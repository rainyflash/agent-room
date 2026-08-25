use std::env;

use agent_room_application::ports::{
    ModerationActionReservationOutcome, ModerationAuthority, ModerationReportPolicy,
    ModerationReportSubmissionOutcome, ModerationRepository, PrivateRoomSnapshot, PrivateRoomStore,
};
use agent_room_domain::{
    ids::{
        AuditEventId, ModerationActionId, ModerationCaseId, PrincipalId, RoomCatalogId,
        RoomInstanceId,
    },
    moderation::{
        ModerationAction, ModerationActionKind, ModerationActionStatus, ModerationAuditEvent,
        ModerationAuditOutcome, ModerationCase, ModerationEvidence, ModerationReason,
        ModerationRole, ModerationTarget, ModerationTargetKind,
    },
    private_rooms::PrivateRoom,
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
    },
    time::{DurationMillis, UtcMillis},
};
use agent_room_postgres_adapter::{PostgresRepositories, run_migrations};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

struct TestDatabase {
    migration: PgPool,
    runtime: PgPool,
}

impl TestDatabase {
    async fn connect() -> Self {
        let migration = connect_pool(&required_url("AGENT_ROOM_TEST_MIGRATION_DATABASE_URL")).await;
        run_migrations(&migration).await.expect("迁移必须成功");
        let runtime = connect_pool(&required_url("AGENT_ROOM_TEST_RUNTIME_DATABASE_URL")).await;
        Self { migration, runtime }
    }

    async fn close(self) {
        self.runtime.close().await;
        self.migration.close().await;
    }
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 举报限流原子生效且审计不保存未提交正文() {
    let database = TestDatabase::connect().await;
    let reporter = seed_principal(&database.runtime, "reporter").await;
    let target = seed_principal(&database.runtime, "reported").await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let policy = ModerationReportPolicy {
        maximum_reports: 2,
        window: DurationMillis::new(60_000).expect("限速窗口有效"),
    };
    let first = report(reporter, target, 0);
    let second = report(reporter, target, 1);
    let third = report(reporter, target, 2);
    let first_audit = report_audit(&first);
    let second_audit = report_audit(&second);

    let (first_result, second_result) = tokio::join!(
        ModerationRepository::submit_case(&repositories, &first, &first_audit, policy,),
        ModerationRepository::submit_case(&repositories, &second, &second_audit, policy,),
    );
    assert!(matches!(
        first_result.expect("并发举报一应成功"),
        ModerationReportSubmissionOutcome::Created(_)
    ));
    assert!(matches!(
        second_result.expect("并发举报二应成功"),
        ModerationReportSubmissionOutcome::Created(_)
    ));
    assert!(matches!(
        ModerationRepository::submit_case(&repositories, &third, &report_audit(&third), policy,)
            .await
            .expect("限速是业务结果而非仓储故障"),
        ModerationReportSubmissionOutcome::RateLimited { .. }
    ));

    let stored = ModerationRepository::list_cases_for_reporter(&repositories, reporter)
        .await
        .expect("案件列表应可读取");
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().all(|case| {
        case.evidence().reporter_submitted_excerpt().is_none()
            && case.evidence().end_to_end_encrypted()
    }));
    let audit_metadata: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT metadata FROM agent_room.audit_event WHERE action = 'moderation.report.created'",
    )
    .fetch_all(&database.runtime)
    .await
    .expect("最小审计元数据应可读取");
    assert_eq!(
        audit_metadata,
        vec![serde_json::json!({}), serde_json::json!({})]
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 治理权限动作撤销和审计访问都读取当前事实() {
    let database = TestDatabase::connect().await;
    let owner = seed_principal(&database.runtime, "room-owner").await;
    let target = seed_principal(&database.runtime, "room-target").await;
    let auditor = seed_principal(&database.runtime, "auditor").await;
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    PrivateRoomStore::create(
        &PostgresRepositories::new(database.runtime.clone()),
        &private_room_snapshot(catalog_id, owner),
        time(0),
    )
    .await
    .expect("私人房间夹具应创建");
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let target_reference =
        ModerationTarget::new(ModerationTargetKind::Principal, target.to_string())
            .expect("主体目标有效");

    let room_case = room_report(owner, target, catalog_id);
    assert!(matches!(
        ModerationRepository::submit_case(
            &repositories,
            &room_case,
            &report_audit(&room_case),
            ModerationReportPolicy {
                maximum_reports: 10,
                window: DurationMillis::new(60_000).expect("限速窗口有效"),
            },
        )
        .await
        .expect("房间举报应入库"),
        ModerationReportSubmissionOutcome::Created(_)
    ));
    let visible_cases = ModerationRepository::list_room_cases(&repositories, catalog_id)
        .await
        .expect("房间案件队列应可读取");
    assert_eq!(visible_cases, vec![room_case]);

    let authority =
        ModerationAuthority::inspect_room(&repositories, owner, catalog_id, &target_reference)
            .await
            .expect("当前房间权限应可读取")
            .expect("活跃房间应存在");
    assert_eq!(authority.role, ModerationRole::RoomManager);
    assert!(authority.target_matrix_user_id.is_some());

    apply_and_reverse_action(&repositories, owner, catalog_id, target_reference).await;
    verify_operator_roles(&database, &repositories, owner, auditor).await;
    verify_audit_is_append_only(&database.runtime).await;

    let missing = ModerationAuthority::inspect_room(
        &repositories,
        owner,
        catalog_id,
        &ModerationTarget::new(ModerationTargetKind::Principal, Uuid::now_v7().to_string())
            .expect("不存在主体的引用格式仍有效"),
    )
    .await
    .expect("不存在目标应是业务结果");
    assert!(missing.is_none());

    database.close().await;
}

async fn apply_and_reverse_action(
    repositories: &PostgresRepositories,
    owner: PrincipalId,
    catalog_id: RoomCatalogId,
    target_reference: ModerationTarget,
) {
    let mut action = ModerationAction::reserve(
        ModerationActionId::from_uuid(Uuid::now_v7()),
        None,
        owner,
        catalog_id,
        ModerationActionKind::Mute,
        target_reference.clone(),
        ModerationReason::Harassment,
        time(10),
        None,
    )
    .expect("治理动作有效");
    assert!(matches!(
        ModerationRepository::reserve_action(
            repositories,
            &action,
            &action_audit(
                &action,
                "moderation.action.requested",
                ModerationAuditOutcome::Allowed
            ),
        )
        .await
        .expect("动作预留应成功"),
        ModerationActionReservationOutcome::Reserved(_)
    ));
    action.mark_applied().expect("动作应进入已应用状态");
    ModerationRepository::finalize_action(
        repositories,
        &action,
        &action_audit(
            &action,
            "moderation.action.applied",
            ModerationAuditOutcome::Allowed,
        ),
    )
    .await
    .expect("已应用终态应提交");
    action.reverse(time(20)).expect("动作应可撤销");
    let reversed = ModerationRepository::finalize_action(
        repositories,
        &action,
        &action_audit(
            &action,
            "moderation.action.reversed",
            ModerationAuditOutcome::Allowed,
        ),
    )
    .await
    .expect("撤销终态应提交");
    assert_eq!(reversed.status(), ModerationActionStatus::Reversed);
    assert_eq!(
        ModerationRepository::list_audit(repositories, Some(catalog_id), 20)
            .await
            .expect("房间审计应可读取")
            .len(),
        3
    );
}

async fn verify_operator_roles(
    database: &TestDatabase,
    repositories: &PostgresRepositories,
    owner: PrincipalId,
    auditor: PrincipalId,
) {
    let runtime_role_write = sqlx::query(
        r"INSERT INTO agent_room.moderation_operator (
               principal_id, role, granted_by, granted_at
           ) VALUES ($1, 'moderator', $2, now())",
    )
    .bind(auditor.as_uuid())
    .bind(owner.as_uuid())
    .execute(&database.runtime)
    .await;
    assert!(
        runtime_role_write.is_err(),
        "运行时账号不得给自己授予平台角色"
    );
    sqlx::query(
        r"INSERT INTO agent_room.moderation_operator (
               principal_id, role, granted_by, granted_at
           ) VALUES ($1, 'audit_reader', $2, now())",
    )
    .bind(auditor.as_uuid())
    .bind(owner.as_uuid())
    .execute(&database.migration)
    .await
    .expect("迁移/运维账号可授予独立审计角色");
    assert_eq!(
        ModerationAuthority::platform_role(repositories, auditor)
            .await
            .expect("审计角色应可读取"),
        ModerationRole::AuditReader
    );
    sqlx::query(
        "UPDATE agent_room.moderation_operator SET revoked_at = now() WHERE principal_id = $1",
    )
    .bind(auditor.as_uuid())
    .execute(&database.migration)
    .await
    .expect("运维账号可撤销角色");
    assert_eq!(
        ModerationAuthority::platform_role(repositories, auditor)
            .await
            .expect("撤销后应读取最新事实"),
        ModerationRole::None
    );
}

async fn verify_audit_is_append_only(pool: &PgPool) {
    let audit_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM agent_room.audit_event WHERE action = 'moderation.action.applied' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("动作审计应存在");
    let tamper = sqlx::query("UPDATE agent_room.audit_event SET outcome = 'failed' WHERE id = $1")
        .bind(audit_id)
        .execute(pool)
        .await;
    assert!(tamper.is_err(), "追加式审计不得被运行时账号篡改");
}

fn report(reporter: PrincipalId, target: PrincipalId, offset: i64) -> ModerationCase {
    ModerationCase::open(
        ModerationCaseId::from_uuid(Uuid::now_v7()),
        reporter,
        ModerationTarget::new(ModerationTargetKind::Principal, target.to_string())
            .expect("举报目标有效"),
        ModerationReason::Harassment,
        "仅由举报者输入的案件说明",
        ModerationEvidence::new(None, None, None, true).expect("引用式证据有效"),
        time(offset),
    )
    .expect("举报案件有效")
}

fn room_report(
    reporter: PrincipalId,
    target: PrincipalId,
    catalog_id: RoomCatalogId,
) -> ModerationCase {
    ModerationCase::open(
        ModerationCaseId::from_uuid(Uuid::now_v7()),
        reporter,
        ModerationTarget::new(ModerationTargetKind::Principal, target.to_string())
            .expect("房间举报目标有效"),
        ModerationReason::Harassment,
        "仅由举报者输入的房间案件说明",
        ModerationEvidence::new(Some(catalog_id), None, None, true).expect("房间引用式证据有效"),
        time(100),
    )
    .expect("房间举报案件有效")
}

fn report_audit(case: &ModerationCase) -> ModerationAuditEvent {
    ModerationAuditEvent::new(
        AuditEventId::from_uuid(Uuid::now_v7()),
        case.created_at(),
        case.reporter_principal_id(),
        "moderation.report.created",
        case.target().clone(),
        ModerationAuditOutcome::Allowed,
        Some(case.reason()),
        AuditEventId::from_uuid(case.id().as_uuid()),
        case.evidence().room_catalog_id(),
    )
    .expect("举报审计有效")
}

fn action_audit(
    action: &ModerationAction,
    code: &str,
    outcome: ModerationAuditOutcome,
) -> ModerationAuditEvent {
    ModerationAuditEvent::new(
        AuditEventId::from_uuid(Uuid::now_v7()),
        action.reversed_at().unwrap_or(action.starts_at()),
        action.actor_principal_id(),
        code,
        action.target().clone(),
        outcome,
        Some(action.reason()),
        AuditEventId::from_uuid(action.id().as_uuid()),
        Some(action.room_catalog_id()),
    )
    .expect("动作审计有效")
}

fn private_room_snapshot(catalog_id: RoomCatalogId, owner: PrincipalId) -> PrivateRoomSnapshot {
    let catalog = RoomCatalog::new(
        catalog_id,
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: "治理测试室".to_owned(),
            description: "真实权限矩阵测试".to_owned(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(owner),
            visibility: RoomCatalogVisibility::Private,
            retention_days: Some(30),
            status: RoomCatalogStatus::Active,
        },
    )
    .expect("私人目录有效");
    let instance_id = RoomInstanceId::from_uuid(Uuid::now_v7());
    let instance = RoomInstance::restore(
        instance_id,
        RoomInstanceFields {
            catalog_id,
            matrix_room_id: MatrixRoomReference::new(format!(
                "!moderation{}:matrix.test",
                instance_id.as_uuid().simple()
            ))
            .expect("Matrix 房间标识有效"),
            region: None,
            capacity: RoomCapacity::new(8, 16).expect("容量有效"),
            projected_member_count: 1,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .expect("房间实例有效");
    PrivateRoomSnapshot::new(catalog, instance, PrivateRoom::create(catalog_id, owner))
        .expect("私人房间快照有效")
}

async fn seed_principal(pool: &PgPool, suffix: &str) -> PrincipalId {
    let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    sqlx::query(
        r"INSERT INTO agent_room.principal (
               id, oidc_issuer, oidc_subject, matrix_user_id, display_name,
               locale, status, created_at, updated_at, version
           ) VALUES (
               $1, 'https://issuer.test', $2, $3, $4, 'zh-CN', 'active',
               to_timestamp($5::double precision / 1000.0),
               to_timestamp($5::double precision / 1000.0), 0
           )",
    )
    .bind(principal_id.as_uuid())
    .bind(format!(
        "subject-{suffix}-{}",
        principal_id.as_uuid().simple()
    ))
    .bind(format!(
        "@{suffix}-{}:matrix.test",
        principal_id.as_uuid().simple()
    ))
    .bind(format!("测试主体 {suffix}"))
    .bind(time(0).value())
    .execute(pool)
    .await
    .expect("主体写入应成功");
    principal_id
}

fn time(offset: i64) -> UtcMillis {
    UtcMillis::new(1_800_000_000_000 + offset).expect("测试时间有效")
}

fn required_url(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("缺少真实数据库测试配置 {name}"))
}

async fn connect_pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .min_connections(0)
        .max_connections(8)
        .connect(url)
        .await
        .expect("真实 PostgreSQL 必须可连接")
}
