use std::{cell::Cell, env};

use agent_room_application::ports::{
    MatrixRoomId, MatrixUserId, ModerationActionReservationOutcome, ModerationAuthority,
    ModerationExpiryClaim, ModerationExpiryRepository, ModerationExpiryReschedule,
    ModerationMuteLedger, ModerationMuteLock, ModerationMuteStanding, ModerationReportPolicy,
    ModerationReportSubmissionOutcome, ModerationRepository, ModerationRoomContext,
    PrivateRoomSnapshot, PrivateRoomStore, StandingModerationSource,
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

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 公开大厅的治理拿到全部活跃分片_最活跃的在前() {
    let database = TestDatabase::connect().await;
    let moderator = seed_principal(&database.runtime, "lobby-moderator").await;
    let target = seed_principal(&database.runtime, "lobby-target").await;
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let shards = seed_public_lobby(
        &database.runtime,
        catalog_id,
        &[
            ("quiet", "active", "1.5", 3),
            ("busy", "active", "9.25", 40),
            ("broken", "failed", "99", 0),
            ("tied", "active", "1.5", 7),
        ],
    )
    .await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let person = ModerationTarget::new(ModerationTargetKind::Principal, target.to_string())
        .expect("主体目标有效");
    // 和围观选分片一样：先比活跃度，再比人数；没在用的分片不算。
    let expected = [shards[1].as_str(), shards[3].as_str(), shards[0].as_str()];

    let ordinary = ModerationAuthority::inspect_room(&repositories, moderator, catalog_id, &person)
        .await
        .expect("公开大厅的治理上下文应可读取")
        .expect("有活跃分片的公开大厅应存在");
    assert_eq!(ordinary.role, ModerationRole::None);
    assert_eq!(ordinary.room_kind, RoomCatalogKind::PublicLobby);
    assert_eq!(room_ids(&ordinary), expected);
    assert!(ordinary.target_matrix_user_id.is_some());

    sqlx::query(
        r"INSERT INTO agent_room.moderation_operator (
               principal_id, role, granted_by, granted_at
           ) VALUES ($1, 'moderator', $1, now())",
    )
    .bind(moderator.as_uuid())
    .execute(&database.migration)
    .await
    .expect("运维账号可授予平台管理员");
    let moderating =
        ModerationAuthority::inspect_room(&repositories, moderator, catalog_id, &person)
            .await
            .expect("平台管理员的治理上下文应可读取")
            .expect("有活跃分片的公开大厅应存在");
    assert_eq!(moderating.role, ModerationRole::PlatformModerator);
    assert_eq!(room_ids(&moderating), expected);

    sqlx::query(
        "UPDATE agent_room.room_instance SET state = 'draining' \
         WHERE catalog_entry_id = $1 AND state = 'active'",
    )
    .bind(catalog_id.as_uuid())
    .execute(&database.runtime)
    .await
    .expect("可以让分片都不再接人");
    assert!(
        ModerationAuthority::inspect_room(&repositories, moderator, catalog_id, &person)
            .await
            .expect("没有活跃分片也是业务结果")
            .is_none(),
        "没有活跃分片时和以前一样当作找不到"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 新开分片要补的只有此刻生效的禁言和封禁() {
    use ModerationActionKind::{Ban, Kick, Mute};
    let database = TestDatabase::connect().await;
    let pool = &database.runtime;
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let other_lobby = RoomCatalogId::from_uuid(Uuid::now_v7());
    seed_public_lobby(pool, catalog_id, &[("live", "active", "1", 1)]).await;
    seed_public_lobby(pool, other_lobby, &[("other", "active", "1", 1)]).await;
    let moderator = seed_principal(pool, "carry-moderator").await;
    let lobby = Ledger::new(pool, moderator, catalog_id);
    let banned = seed_principal(pool, "banned").await;
    let muted = seed_principal(pool, "muted").await;
    let shouting = seed_principal(pool, "shouting").await;
    let expired = seed_principal(pool, "expired").await;
    let forgiven = seed_principal(pool, "forgiven").await;
    let failed = seed_principal(pool, "failed").await;
    let pending = seed_principal(pool, "pending").await;

    lobby.record(Ban, banned, Ending::Applied, None).await;
    // 禁言到 `time(500)`，读的时候是 `time(100)`：还没到期。
    lobby
        .record(Mute, muted, Ending::Applied, Some(time(500)))
        .await;
    // 引用不是规范写法（大写）也认得出是谁：治理当初就是按 UUID 解析放行的。
    lobby
        .record_reference(Ban, &shouting.to_string().to_uppercase())
        .await;
    lobby
        .record(Mute, expired, Ending::Applied, Some(time(50)))
        .await;
    lobby.record(Ban, forgiven, Ending::Reversed, None).await;
    lobby.record(Ban, failed, Ending::Failed, None).await;
    lobby.record(Ban, pending, Ending::Pending, None).await;
    lobby.record(Kick, banned, Ending::Applied, None).await;
    lobby.hide("$spam:matrix.test").await;
    Ledger::new(pool, moderator, other_lobby)
        .record(Ban, muted, Ending::Applied, None)
        .await;

    let standing = StandingModerationSource::standing_person_actions(
        &PostgresRepositories::new(pool.clone()),
        catalog_id,
        time(100),
    )
    .await
    .expect("要补的治理应可读取");
    let found: Vec<(ModerationActionKind, String)> = standing
        .iter()
        .map(|item| {
            (
                item.action.kind(),
                item.target_matrix_user_id.as_str().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            (Ban, matrix_user(pool, banned).await),
            (Mute, matrix_user(pool, muted).await),
            (Ban, matrix_user(pool, shouting).await),
        ],
        "只有此刻生效的禁言和封禁，先做的在前；踢出、隐藏和别的大厅的都不算"
    );

    database.close().await;
}

/// 治理动作最后落成什么样。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ending {
    Applied,
    Reversed,
    Failed,
    Pending,
}

/// 往一个大厅里记治理：按真实的预留、落下、撤销走一遍写进库。
struct Ledger {
    repositories: PostgresRepositories,
    moderator: PrincipalId,
    catalog_id: RoomCatalogId,
    /// 下一步在 `time(这个数)` 做，一步比一步晚一毫秒，都早于读的时候 `time(100)`。
    next: Cell<i64>,
}

impl Ledger {
    fn new(pool: &PgPool, moderator: PrincipalId, catalog_id: RoomCatalogId) -> Self {
        Self {
            repositories: PostgresRepositories::new(pool.clone()),
            moderator,
            catalog_id,
            next: Cell::new(10),
        }
    }

    fn tick(&self) -> UtcMillis {
        let offset = self.next.get();
        self.next.set(offset + 1);
        time(offset)
    }

    async fn record(
        &self,
        kind: ModerationActionKind,
        target: PrincipalId,
        ending: Ending,
        expires_at: Option<UtcMillis>,
    ) {
        let target = ModerationTarget::new(ModerationTargetKind::Principal, target.to_string())
            .expect("主体目标有效");
        let mut action = self.reserve(kind, target, expires_at).await;
        match ending {
            Ending::Pending => {}
            Ending::Failed => {
                action
                    .mark_failed("matrix.unavailable")
                    .expect("动作可记成失败");
                self.finalize(&action, "moderation.action.failed").await;
            }
            Ending::Applied | Ending::Reversed => {
                action.mark_applied().expect("动作可记成已落下");
                self.finalize(&action, "moderation.action.applied").await;
                if ending == Ending::Reversed {
                    action.reverse(self.tick()).expect("动作可撤销");
                    self.finalize(&action, "moderation.action.reversed").await;
                }
            }
        }
    }

    /// 一条已经落下、引用照原样写的管人治理。
    async fn record_reference(&self, kind: ModerationActionKind, reference: &str) {
        let target = ModerationTarget::new(ModerationTargetKind::Principal, reference)
            .expect("主体目标有效");
        let mut action = self.reserve(kind, target, None).await;
        action.mark_applied().expect("动作可记成已落下");
        self.finalize(&action, "moderation.action.applied").await;
    }

    /// 一条已经落下的隐藏：管的是消息，不是人。
    async fn hide(&self, event_id: &str) {
        let target =
            ModerationTarget::new(ModerationTargetKind::Event, event_id).expect("事件目标有效");
        let mut hide = self.reserve(ModerationActionKind::Hide, target, None).await;
        hide.mark_applied().expect("隐藏可记成已落下");
        self.finalize(&hide, "moderation.action.applied").await;
    }

    async fn reserve(
        &self,
        kind: ModerationActionKind,
        target: ModerationTarget,
        expires_at: Option<UtcMillis>,
    ) -> ModerationAction {
        let action = ModerationAction::reserve(
            ModerationActionId::from_uuid(Uuid::now_v7()),
            None,
            self.moderator,
            self.catalog_id,
            kind,
            target,
            ModerationReason::Harassment,
            self.tick(),
            expires_at,
        )
        .expect("治理动作有效");
        let requested = action_audit(
            &action,
            "moderation.action.requested",
            ModerationAuditOutcome::Allowed,
        );
        ModerationRepository::reserve_action(&self.repositories, &action, &requested)
            .await
            .expect("动作预留应成功");
        action
    }

    async fn finalize(&self, action: &ModerationAction, code: &str) {
        let outcome = if action.status() == ModerationActionStatus::Failed {
            ModerationAuditOutcome::Failed
        } else {
            ModerationAuditOutcome::Allowed
        };
        ModerationRepository::finalize_action(
            &self.repositories,
            action,
            &action_audit(action, code, outcome),
        )
        .await
        .expect("动作终态应提交");
    }
}

async fn matrix_user(pool: &PgPool, principal: PrincipalId) -> String {
    sqlx::query_scalar("SELECT matrix_user_id FROM agent_room.principal WHERE id = $1")
        .bind(principal.as_uuid())
        .fetch_one(pool)
        .await
        .expect("主体的 Matrix 账号应可读取")
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 到期解除一个一个地领_租约没过不再领_排下次只认自己领的那次() {
    use ModerationActionKind::{Ban, Mute};
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    drain_due(&repositories).await;
    let moderator = seed_principal(&database.runtime, "expiry-moderator").await;
    let first = person(seed_principal(&database.runtime, "expiry-first").await);
    let second = person(seed_principal(&database.runtime, "expiry-second").await);
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    seed_public_lobby(
        &database.runtime,
        catalog_id,
        &[("expiry", "active", "1", 1)],
    )
    .await;
    let room = (&repositories, moderator, catalog_id);
    let early = applied_action(room, Mute, &first, Some(time(1_000))).await;
    let tied = applied_action(room, Mute, &second, Some(time(1_000))).await;
    let late = applied_action(room, Mute, &first, Some(time(5_000))).await;
    applied_action(room, Ban, &second, None).await;
    reserved_action(room, Mute, &second, Some(time(1_000))).await;

    // 两秒时到期的只有已生效的那两个，同一时刻到期的按 ID 排；没到期的、没期限的、还没生效的都不领。
    let claimed = claim(&repositories, time(2_000))
        .await
        .expect("先到期的先领");
    assert_eq!(claim_facts(&claimed), (early.id(), 1, None));
    assert_eq!(claim_id(&repositories, time(2_000)).await, Some(tied.id()));
    assert_eq!(
        claim_id(&repositories, time(2_000)).await,
        None,
        "租约没过的不再领"
    );
    assert_eq!(claim_id(&repositories, time(10_000)).await, Some(late.id()));

    // 撤不成：二十秒时再试。排下次只认自己领的那次。
    let retry = ModerationExpiryReschedule::Retry {
        at: time(20_000),
        failure_code: "matrix.unavailable",
    };
    assert!(reschedule(&repositories, &claimed, retry).await);
    let stale = ModerationExpiryClaim {
        attempt: 0,
        ..claimed.clone()
    };
    let defer = ModerationExpiryReschedule::Defer { at: time(0) };
    assert!(
        !reschedule(&repositories, &stale, defer).await,
        "不是这一次领的改不动"
    );
    assert_eq!(
        claim_id(&repositories, time(19_999)).await,
        None,
        "退避期间不领"
    );
    let again = claim(&repositories, time(20_000))
        .await
        .expect("退避过了接着领");
    assert_eq!(
        claim_facts(&again),
        (early.id(), 2, Some("matrix.unavailable"))
    );
    assert!(
        !reschedule(&repositories, &claimed, retry).await,
        "被重新领走以后，前一次领的改不动"
    );

    // 定不下来：领过的次数退回去，失败码照旧留着。
    let defer = ModerationExpiryReschedule::Defer { at: time(30_000) };
    assert!(reschedule(&repositories, &again, defer).await);
    let deferred = claim(&repositories, time(30_000))
        .await
        .expect("下一轮再看");
    assert_eq!(
        claim_facts(&deferred),
        (early.id(), 2, Some("matrix.unavailable"))
    );

    let mut expired = deferred.action.clone();
    expired.expire(time(30_000)).expect("到期的动作可以解除");
    finish(&repositories, &expired, "moderation.action.expired").await;
    assert!(
        !reschedule(&repositories, &deferred, retry).await,
        "已经解除的不再排"
    );
    let later = [
        claim_id(&repositories, time(200_000)).await,
        claim_id(&repositories, time(200_000)).await,
        claim_id(&repositories, time(200_000)).await,
    ];
    assert_eq!(
        later,
        [Some(tied.id()), Some(late.id()), None],
        "解除了的不再领"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 两个实例同时领同一个到期的动作_只有一个领到() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let other_instance = PostgresRepositories::new(database.runtime.clone());
    drain_due(&repositories).await;
    let moderator = seed_principal(&database.runtime, "race-moderator").await;
    let target = person(seed_principal(&database.runtime, "race-target").await);
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    seed_public_lobby(&database.runtime, catalog_id, &[("race", "active", "1", 1)]).await;
    let mute = applied_action(
        (&repositories, moderator, catalog_id),
        ModerationActionKind::Mute,
        &target,
        Some(time(1_000)),
    )
    .await;

    for _ in 0..5 {
        let (left, right) = tokio::join!(
            claim(&repositories, time(2_000)),
            claim(&other_instance, time(2_000))
        );
        let claimed: Vec<_> = [left, right].into_iter().flatten().collect();
        assert_eq!(claimed.len(), 1, "同一个只有一个实例领到");
        assert_eq!(claimed[0].action.id(), mute.id());
        // 放回去再抢一次。
        let defer = ModerationExpiryReschedule::Defer { at: time(0) };
        assert!(reschedule(&repositories, &claimed[0], defer).await);
    }

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 还有没有别的生效的同类动作按_uuid_认人() {
    use ModerationActionKind::{Ban, Mute};
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let moderator = seed_principal(&database.runtime, "effective-moderator").await;
    let first_id = seed_principal(&database.runtime, "effective-first").await;
    let first = person(first_id);
    let second = person(seed_principal(&database.runtime, "effective-second").await);
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    seed_public_lobby(
        &database.runtime,
        catalog_id,
        &[("effective", "active", "1", 1)],
    )
    .await;
    let room = (&repositories, moderator, catalog_id);
    let early = applied_action(room, Mute, &first, Some(time(1_000))).await;
    let late = applied_action(room, Mute, &first, Some(time(5_000))).await;
    let lonely = applied_action(room, Mute, &second, Some(time(1_000))).await;
    applied_action(room, Ban, &second, None).await;
    reserved_action(room, Mute, &second, None).await;

    // 同一个人身上还有一条五秒才到期的禁言在生效；另一个人身上只有封禁和还没生效的禁言，都不算。
    assert!(has_other_effective(&repositories, &early, time(2_000)).await);
    assert!(!has_other_effective(&repositories, &lonely, time(2_000)).await);
    assert!(
        !has_other_effective(&repositories, &late, time(6_000)).await,
        "到了期限的那条不算还在生效"
    );

    // 管人的动作按 UUID 认人：大写、不带横线的写法也是同一个人。
    let shouted = ModerationTarget::new(
        ModerationTargetKind::Principal,
        first_id.as_uuid().simple().to_string().to_uppercase(),
    )
    .expect("大写、不带横线的 UUID 也是有效目标");
    let timed_ban = applied_action(room, Ban, &first, Some(time(1_000))).await;
    assert!(!has_other_effective(&repositories, &timed_ban, time(2_000)).await);
    applied_action(room, Ban, &shouted, None).await;
    assert!(has_other_effective(&repositories, &timed_ban, time(2_000)).await);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 读一个人的禁言_只读这个房间这个人已经落下和正在落的_大写写法也认() {
    use ModerationActionKind::{Ban, Mute};
    let database = TestDatabase::connect().await;
    let pool = &database.runtime;
    let repositories = PostgresRepositories::new(pool.clone());
    let moderator = seed_principal(pool, "standing-moderator").await;
    let target_id = seed_principal(pool, "standing-target").await;
    let target = person(target_id);
    let shouted = ModerationTarget::new(
        ModerationTargetKind::Principal,
        target_id.as_uuid().simple().to_string().to_uppercase(),
    )
    .expect("大写、不带横线的 UUID 也是有效目标");
    let other = person(seed_principal(pool, "standing-other").await);
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let other_lobby = RoomCatalogId::from_uuid(Uuid::now_v7());
    let shards = seed_public_lobby(
        pool,
        catalog_id,
        &[
            ("quiet", "active", "1.5", 3),
            ("busy", "active", "9.25", 40),
            ("gone", "draining", "99", 0),
        ],
    )
    .await;
    seed_public_lobby(pool, other_lobby, &[("elsewhere", "active", "1", 1)]).await;
    let room = (&repositories, moderator, catalog_id);
    let applied = applied_action(room, Mute, &target, Some(time(1_000))).await;
    let landing = reserved_action(room, Mute, &shouted, None).await;
    let mut lifted = applied_action(room, Mute, &target, None).await;
    lifted.reverse(time(100)).expect("可以撤回");
    finish(&repositories, &lifted, "moderation.action.reversed").await;
    let mut broken = reserved_action(room, Mute, &target, None).await;
    broken
        .mark_failed("matrix.unavailable")
        .expect("可以记成失败");
    finish(&repositories, &broken, "moderation.action.failed").await;
    applied_action(room, Ban, &target, None).await;
    applied_action(room, Mute, &other, None).await;
    applied_action((&repositories, moderator, other_lobby), Mute, &target, None).await;

    for asked in [&target, &shouted] {
        let standing = mute_standing(&repositories, catalog_id, asked)
            .await
            .expect("目录存在");
        assert_eq!(standing.room_kind, RoomCatalogKind::PublicLobby);
        assert_eq!(
            standing
                .matrix_room_ids
                .iter()
                .map(MatrixRoomId::as_str)
                .collect::<Vec<_>>(),
            [shards[1].as_str(), shards[0].as_str()],
            "活跃分片，最活跃的在前"
        );
        assert_eq!(
            standing
                .target_matrix_user_id
                .as_ref()
                .map(MatrixUserId::as_str),
            Some(matrix_user(pool, target_id).await.as_str())
        );
        assert!(standing.may_speak, "公开大厅谁都能说话");
        assert_eq!(
            standing
                .mutes
                .iter()
                .map(ModerationAction::id)
                .collect::<Vec<_>>(),
            [applied.id(), landing.id()],
            "撤回的、失败的、封禁、别人的、别的大厅的都不算"
        );
    }
    assert!(
        mute_standing(
            &repositories,
            RoomCatalogId::from_uuid(Uuid::now_v7()),
            &target
        )
        .await
        .is_none(),
        "目录不存在"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 读一个人的禁言_私人房间按成员状态和发言权位算他能不能说() {
    let database = TestDatabase::connect().await;
    let pool = &database.runtime;
    let repositories = PostgresRepositories::new(pool.clone());
    let owner = seed_principal(pool, "speak-owner").await;
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    PrivateRoomStore::create(
        &repositories,
        &private_room_snapshot(catalog_id, owner),
        time(0),
    )
    .await
    .expect("私人房间夹具应创建");
    let mut expected = vec![(owner, true)];
    for (suffix, status, bits, may_speak) in [
        ("speak-speaker", "joined", 3, true),
        ("speak-watcher", "joined", 1, false),
        ("speak-invited", "invited", 3, true),
        ("speak-removed", "removed", 0, false),
    ] {
        let member = seed_principal(pool, suffix).await;
        seed_membership(pool, catalog_id, member, status, bits).await;
        expected.push((member, may_speak));
    }
    expected.push((seed_principal(pool, "speak-stranger").await, false));

    for (member, may_speak) in expected {
        let standing = mute_standing(&repositories, catalog_id, &person(member))
            .await
            .expect("目录存在");
        assert_eq!(standing.room_kind, RoomCatalogKind::PrivateRoom);
        assert_eq!(standing.matrix_room_ids.len(), 1);
        assert_eq!(standing.may_speak, may_speak, "{member}");
    }

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 禁言锁同一个房间里同一个人一次只有一个拿得到() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let person_id = PrincipalId::from_uuid(Uuid::now_v7());
    let target = person(person_id);
    let shouted = ModerationTarget::new(
        ModerationTargetKind::Principal,
        person_id.as_uuid().simple().to_string().to_uppercase(),
    )
    .expect("大写、不带横线的 UUID 也是有效目标");

    let held = try_lock(&repositories, catalog_id, &target)
        .await
        .expect("没人拿着时拿得到");
    assert!(
        try_lock(&repositories, catalog_id, &shouted)
            .await
            .is_none(),
        "写法不一样也是同一个人"
    );
    let someone_else = PrincipalId::from_uuid(Uuid::now_v7());
    let elsewhere = RoomCatalogId::from_uuid(Uuid::now_v7());
    for (room, who) in [
        (catalog_id, person(someone_else)),
        (elsewhere, target.clone()),
    ] {
        try_lock(&repositories, room, &who)
            .await
            .expect("换个人、换个房间互不影响")
            .release()
            .await;
    }

    held.release().await;
    try_lock(&repositories, catalog_id, &shouted)
        .await
        .expect("放掉以后又能拿")
        .release()
        .await;

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 读一个人的禁言_账号删了照样给出_matrix_账号_分片都不接人时给空的() {
    let database = TestDatabase::connect().await;
    let pool = &database.runtime;
    let repositories = PostgresRepositories::new(pool.clone());
    let target_id = seed_principal(pool, "standing-deleted").await;
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    seed_public_lobby(pool, catalog_id, &[("closing", "active", "1", 1)]).await;
    sqlx::query("UPDATE agent_room.principal SET status = 'deleted' WHERE id = $1")
        .bind(target_id.as_uuid())
        .execute(pool)
        .await
        .expect("可以把被禁言的账号标成已删除");
    sqlx::query(
        "UPDATE agent_room.room_instance SET state = 'draining'          WHERE catalog_entry_id = $1 AND state = 'active'",
    )
    .bind(catalog_id.as_uuid())
    .execute(pool)
    .await
    .expect("可以让分片都不再接人");

    let standing = mute_standing(&repositories, catalog_id, &person(target_id))
        .await
        .expect("目录存在");
    assert!(standing.matrix_room_ids.is_empty(), "没有活跃分片时给空的");
    assert_eq!(
        standing
            .target_matrix_user_id
            .as_ref()
            .map(MatrixUserId::as_str),
        Some(matrix_user(pool, target_id).await.as_str()),
        "账号删了照样给出它的 Matrix 账号"
    );
    let stranger = person(PrincipalId::from_uuid(Uuid::now_v7()));
    assert!(
        mute_standing(&repositories, catalog_id, &stranger)
            .await
            .expect("目录存在")
            .target_matrix_user_id
            .is_none(),
        "没有这个人就没有 Matrix 账号"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 到期解除拿到全部活跃分片_账号删了也照样给出它的_matrix_账号() {
    let database = TestDatabase::connect().await;
    let moderator = seed_principal(&database.runtime, "expiry-room-moderator").await;
    let target_id = seed_principal(&database.runtime, "expiry-room-target").await;
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let shards = seed_public_lobby(
        &database.runtime,
        catalog_id,
        &[
            ("quiet", "active", "1.5", 3),
            ("busy", "active", "9.25", 40),
            ("broken", "failed", "99", 0),
        ],
    )
    .await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let room = (&repositories, moderator, catalog_id);
    let target = person(target_id);
    let mute = applied_action(room, ModerationActionKind::Mute, &target, Some(time(1_000))).await;
    sqlx::query("UPDATE agent_room.principal SET status = 'deleted' WHERE id = $1")
        .bind(target_id.as_uuid())
        .execute(&database.runtime)
        .await
        .expect("可以把被禁言的账号标成已删除");

    let context = expiry_room(&repositories, &mute).await;
    assert_eq!(context.role, ModerationRole::None);
    assert_eq!(context.room_kind, RoomCatalogKind::PublicLobby);
    assert_eq!(room_ids(&context), [shards[1].as_str(), shards[0].as_str()]);
    let target_user = format!(
        "@expiry-room-target-{}:matrix.test",
        target_id.as_uuid().simple()
    );
    assert_eq!(
        context
            .target_matrix_user_id
            .as_ref()
            .map(MatrixUserId::as_str),
        Some(target_user.as_str())
    );
    assert!(
        ModerationAuthority::inspect_room(&repositories, moderator, catalog_id, &target)
            .await
            .expect("治理上下文应可读取")
            .is_none(),
        "治理接口只认活跃账号，到期解除不受这个限制"
    );

    let event = ModerationTarget::new(ModerationTargetKind::Event, "$expiry:matrix.test")
        .expect("事件目标有效");
    let hide = applied_action(room, ModerationActionKind::Hide, &event, Some(time(1_000))).await;
    let hidden = expiry_room(&repositories, &hide).await;
    assert_eq!(room_ids(&hidden), [shards[1].as_str(), shards[0].as_str()]);
    assert!(hidden.target_matrix_user_id.is_none());

    sqlx::query(
        "UPDATE agent_room.room_instance SET state = 'draining' \
         WHERE catalog_entry_id = $1 AND state = 'active'",
    )
    .bind(catalog_id.as_uuid())
    .execute(&database.runtime)
    .await
    .expect("可以让分片都不再接人");
    assert!(
        expiry_room(&repositories, &mute)
            .await
            .matrix_room_ids
            .is_empty(),
        "没有活跃分片时给空的，由应用层记成解除"
    );

    database.close().await;
}

/// 治理动作落在哪：仓库、做动作的人、房间。
type ActionRoom<'a> = (&'a PostgresRepositories, PrincipalId, RoomCatalogId);

/// 预留一个从零时开始的治理动作，还没生效。
async fn reserved_action(
    (repositories, actor, catalog_id): ActionRoom<'_>,
    kind: ModerationActionKind,
    target: &ModerationTarget,
    expires_at: Option<UtcMillis>,
) -> ModerationAction {
    let action = ModerationAction::reserve(
        ModerationActionId::from_uuid(Uuid::now_v7()),
        None,
        actor,
        catalog_id,
        kind,
        target.clone(),
        ModerationReason::Spam,
        time(0),
        expires_at,
    )
    .expect("治理动作有效");
    ModerationRepository::reserve_action(
        repositories,
        &action,
        &action_audit(
            &action,
            "moderation.action.requested",
            ModerationAuditOutcome::Allowed,
        ),
    )
    .await
    .expect("动作预留应成功");
    action
}

async fn applied_action(
    room: ActionRoom<'_>,
    kind: ModerationActionKind,
    target: &ModerationTarget,
    expires_at: Option<UtcMillis>,
) -> ModerationAction {
    let mut action = reserved_action(room, kind, target, expires_at).await;
    action.mark_applied().expect("动作应进入已应用状态");
    ModerationRepository::finalize_action(
        room.0,
        &action,
        &action_audit(
            &action,
            "moderation.action.applied",
            ModerationAuditOutcome::Allowed,
        ),
    )
    .await
    .expect("已应用终态应提交")
}

/// 别的用例留下的、早就到期的动作先领走，租约设到很久以后，免得混进这个用例。
async fn drain_due(repositories: &PostgresRepositories) {
    while ModerationExpiryRepository::claim_due_action(
        repositories,
        time(1_000_000),
        time(1_000_000_000_000),
    )
    .await
    .expect("到期动作应可领取")
    .is_some()
    {}
}

/// 领到的是哪个、第几次领、上次撤不成的失败码。
fn claim_facts(claim: &ModerationExpiryClaim) -> (ModerationActionId, u32, Option<&str>) {
    (
        claim.action.id(),
        claim.attempt,
        claim.previous_failure_code.as_deref(),
    )
}

/// 领一个到期的，租约一分钟。
async fn claim(
    repositories: &PostgresRepositories,
    now: UtcMillis,
) -> Option<ModerationExpiryClaim> {
    let lease_until = UtcMillis::new(now.value() + 60_000).expect("租约时间有效");
    ModerationExpiryRepository::claim_due_action(repositories, now, lease_until)
        .await
        .expect("到期动作应可领取")
}

async fn claim_id(
    repositories: &PostgresRepositories,
    now: UtcMillis,
) -> Option<ModerationActionId> {
    claim(repositories, now)
        .await
        .map(|claim| claim.action.id())
}

async fn reschedule(
    repositories: &PostgresRepositories,
    claim: &ModerationExpiryClaim,
    reschedule: ModerationExpiryReschedule,
) -> bool {
    ModerationExpiryRepository::reschedule_expiry(repositories, claim, reschedule)
        .await
        .expect("排下次应能写")
}

async fn mute_standing(
    repositories: &PostgresRepositories,
    catalog_id: RoomCatalogId,
    target: &ModerationTarget,
) -> Option<ModerationMuteStanding> {
    ModerationMuteLedger::mute_standing(repositories, catalog_id, target)
        .await
        .expect("禁言情况应可读取")
}

async fn try_lock(
    repositories: &PostgresRepositories,
    catalog_id: RoomCatalogId,
    target: &ModerationTarget,
) -> Option<Box<dyn ModerationMuteLock>> {
    ModerationMuteLedger::try_lock_mutes(repositories, catalog_id, target)
        .await
        .expect("禁言锁应可拿")
}

/// 把动作改成的终态落库。
async fn finish(repositories: &PostgresRepositories, action: &ModerationAction, code: &str) {
    ModerationRepository::finalize_action(
        repositories,
        action,
        &action_audit(action, code, ModerationAuditOutcome::Allowed),
    )
    .await
    .expect("终态应提交");
}

/// 私人房间里加一个成员：状态和权限位（第 1 位能看，第 2 位能说）。
async fn seed_membership(
    pool: &PgPool,
    catalog_id: RoomCatalogId,
    principal: PrincipalId,
    status: &str,
    permission_bits: i16,
) {
    sqlx::query(
        r"INSERT INTO agent_room.private_room_membership (
              catalog_entry_id, principal_id, membership_status, permission_bits,
              created_at, status_changed_at
          ) VALUES ($1, $2, $3, $4, to_timestamp(1800000000), to_timestamp(1800000000))",
    )
    .bind(catalog_id.as_uuid())
    .bind(principal.as_uuid())
    .bind(status)
    .bind(permission_bits)
    .execute(pool)
    .await
    .expect("私人房间成员应写入");
}

async fn has_other_effective(
    repositories: &PostgresRepositories,
    action: &ModerationAction,
    now: UtcMillis,
) -> bool {
    ModerationExpiryRepository::has_other_effective_action(repositories, action, now)
        .await
        .expect("同类动作应可查询")
}

async fn expiry_room(
    repositories: &PostgresRepositories,
    action: &ModerationAction,
) -> ModerationRoomContext {
    ModerationExpiryRepository::expiry_room(repositories, action)
        .await
        .expect("到期解除的房间应可读取")
        .expect("目录存在")
}

fn person(principal_id: PrincipalId) -> ModerationTarget {
    ModerationTarget::new(ModerationTargetKind::Principal, principal_id.to_string())
        .expect("主体目标有效")
}

fn room_ids(context: &ModerationRoomContext) -> Vec<&str> {
    context
        .matrix_room_ids
        .iter()
        .map(MatrixRoomId::as_str)
        .collect()
}

/// 建一个公开大厅和它的分片：（名字、状态、活跃度、人数），交回各分片的 Matrix 房间 ID。
async fn seed_public_lobby(
    pool: &PgPool,
    catalog_id: RoomCatalogId,
    shards: &[(&str, &str, &str, i32)],
) -> Vec<String> {
    let suffix = catalog_id.as_uuid().simple().to_string();
    sqlx::query(
        r"INSERT INTO agent_room.room_catalog_entry (
              id, kind, slug, name, language, visibility, status, created_at, updated_at
          ) VALUES (
              $1, 'public_lobby', $2, '治理分片测试大厅', 'zh-CN', 'public', 'active',
              to_timestamp(1700000000), to_timestamp(1700000000)
          )",
    )
    .bind(catalog_id.as_uuid())
    .bind(format!("moderation-{}", &suffix[..24]))
    .execute(pool)
    .await
    .expect("公开大厅目录应创建");
    let mut matrix_room_ids = Vec::with_capacity(shards.len());
    for (name, state, activity, members) in shards {
        let matrix_room_id = format!("!{name}-{suffix}:matrix.test");
        sqlx::query(
            r"INSERT INTO agent_room.room_instance (
                  id, catalog_entry_id, matrix_room_id, member_count_projection,
                  activity_score, state, created_at, updated_at
              ) VALUES (
                  $1, $2, $3, $4, $5::numeric, $6,
                  to_timestamp(1700000000), to_timestamp(1700000000)
              )",
        )
        .bind(Uuid::now_v7())
        .bind(catalog_id.as_uuid())
        .bind(&matrix_room_id)
        .bind(members)
        .bind(activity)
        .bind(state)
        .execute(pool)
        .await
        .expect("公开大厅分片应创建");
        matrix_room_ids.push(matrix_room_id);
    }
    matrix_room_ids
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
    let audit = ModerationRepository::list_audit(repositories, Some(catalog_id), 20)
        .await
        .expect("房间审计应可读取");
    assert_eq!(
        audit
            .iter()
            .filter(|event| event.action.starts_with("moderation.action."))
            .count(),
        3,
        "举报审计不应改变动作生命周期的三个审计事实"
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
