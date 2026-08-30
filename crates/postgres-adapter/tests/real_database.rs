use std::{borrow::Cow, collections::BTreeSet, env};

use agent_room_application::{
    persistence::RepositoryErrorKind,
    ports::{
        AccountDeletionRepository, AccountDeletionRequest, AccountDeletionRequestOutcome,
        AccountDeletionStage, AgentCardSnapshotRepository, AgentCreationClaim,
        AgentCreationReservation, AgentCreationWorkflow, AgentInstanceManagementRepository,
        AgentInstanceMatrixCleanupStore, AgentInstanceRegistration,
        AgentInstanceRegistrationTransaction, AgentInstanceRevocationOutcome,
        AgentInstanceRevocationTransaction, AgentInstanceVerificationRepository,
        AgentMembershipChange, AgentMembershipRepository, AgentMembershipTransaction,
        AgentRegistration, AgentRepository, ClaimTargetedHandoff, DeviceRevocationOutcome,
        DeviceRevocationTransaction, DeviceSecurityEvent, HandoffAccessRepository, MatrixUserId,
        OutboxMessage, PrincipalRegistration, PrincipalRepository, QueueTargetedHandoff,
        QueueTargetedHandoffOutcome, RecordTargetedHandoffReceipt, SecretDigest,
        StoredAgentInstanceRegistration, TargetedHandoffReceiptOutcome, TargetedHandoffRepository,
        TargetedHandoffRequestFingerprint,
    },
};
use agent_room_domain::{
    agent_cards::{
        AgentCardCapabilities, AgentCardDigest, AgentCardEndpoint, AgentCardProtocolVersion,
        AgentCardSkill, AgentCardSnapshot, AgentCardSnapshotFields, AgentCardSourceUrl,
        AgentCardTransport, AgentCardVerificationState, AgentEndpointVerificationState,
        NormalizedAgentCard, NormalizedAgentCardFields,
    },
    agents::{
        AdapterBinding, AdapterSubjectHash, Agent, AgentInstance, AgentInstancePublicSigningKey,
        AgentMatrixDeviceId, AgentRole, AgentVisibility,
    },
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    handoff::{
        HandoffContentReference, HandoffFailureCode, HandoffPermission, HandoffPermissions,
        HandoffPurpose, HandoffSourceEventId, TargetedHandoff, TargetedHandoffFields,
        TargetedHandoffStatus,
    },
    identity::Principal,
    ids::{
        AccountDeletionJobId, AdapterBindingId, AgentCardSnapshotId, AgentCreationRequestId,
        AgentId, AgentInstanceId, AgentInstanceRegistrationRequestId, ContentId, DeviceId,
        HandoffId, MessageId, OutboxEventId, PrincipalId,
    },
    rooms::MatrixRoomReference,
    time::{DurationMillis, UtcMillis},
};
use agent_room_postgres_adapter::{PostgresRepositories, run_migrations};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const EXPECTED_TABLES: [&str; 46] = [
    "account_deletion_job",
    "adapter_binding",
    "agent",
    "agent_card_snapshot",
    "agent_creation_request",
    "agent_instance",
    "agent_instance_registration_request",
    "agent_ownership",
    "audit_event",
    "automation_consumption",
    "automation_denial",
    "automation_grant",
    "content_access_policy",
    "content_download_window",
    "content_object",
    "content_upload_request",
    "context_handoff",
    "desktop_authorization_code",
    "device",
    "device_access_token",
    "device_authorization_receipt",
    "device_proof_nonce",
    "device_refresh_token",
    "device_token_family",
    "direct_contact_block",
    "direct_session",
    "federation_governance_audit",
    "federation_governance_rule",
    "federation_peer",
    "matrix_projection_cursor",
    "matrix_projection_event_receipt",
    "moderation_action",
    "moderation_case",
    "moderation_operator",
    "moderation_report_rate",
    "oidc_login_attempt",
    "outbox_event",
    "principal",
    "private_room_membership",
    "private_room_state",
    "room_capacity_reservation",
    "room_catalog_entry",
    "room_instance",
    "room_membership_projection",
    "room_provisioning_job",
    "web_session",
];

struct TestDatabase {
    migration: PgPool,
    runtime: PgPool,
}

impl TestDatabase {
    async fn connect() -> Self {
        let migration_url = required_url("AGENT_ROOM_TEST_MIGRATION_DATABASE_URL");
        let runtime_url = required_url("AGENT_ROOM_TEST_RUNTIME_DATABASE_URL");
        let migration = connect_pool(&migration_url).await;
        run_migrations(&migration).await.expect("首次迁移必须成功");
        let runtime = connect_pool(&runtime_url).await;
        Self { migration, runtime }
    }

    async fn close(self) {
        self.runtime.close().await;
        self.migration.close().await;
    }
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 迁移可重复执行且运行时角色没有建表权限() {
    let database = TestDatabase::connect().await;

    run_migrations(&database.migration)
        .await
        .expect("重复迁移必须幂等");
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = 'agent_room' ORDER BY table_name",
    )
    .fetch_all(&database.migration)
    .await
    .expect("应能读取迁移结果");
    assert_eq!(tables, EXPECTED_TABLES);

    let default_lobby_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS( \
             SELECT 1 FROM agent_room.room_catalog_entry \
             WHERE id = '01a04772-3804-72f9-b1cd-51ca3f730b3d'::uuid \
               AND kind = 'public_lobby' \
               AND visibility = 'public' \
               AND status = 'active' \
         )",
    )
    .fetch_one(&database.runtime)
    .await
    .expect("运行时角色应能读取默认公共大厅");
    assert!(default_lobby_exists, "迁移后必须存在默认公共大厅");

    let current_user: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&database.runtime)
        .await
        .expect("运行时连接应可用");
    assert_eq!(current_user, "agent_room_runtime");
    let can_create: bool =
        sqlx::query_scalar("SELECT has_schema_privilege(current_user, 'agent_room', 'CREATE')")
            .fetch_one(&database.runtime)
            .await
            .expect("应能检查角色权限");
    assert!(!can_create);

    let error = sqlx::query("CREATE TABLE agent_room.runtime_forbidden_table (id integer)")
        .execute(&database.runtime)
        .await
        .expect_err("运行时角色不得执行 DDL");
    assert_eq!(database_code(&error).as_deref(), Some("42501"));

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 账户导出与删除状态机在真实事务中完成() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    let registration = principal_registration(principal_id, "待删除主体");
    PrincipalRepository::create(&repositories, &registration)
        .await
        .expect("主体创建应成功");
    let exported = AccountDeletionRepository::export(&repositories, principal_id, test_time())
        .await
        .expect("账户导出应成功")
        .expect("活动账户必须存在");
    assert_eq!(exported.data["principal"]["displayName"], "待删除主体");

    let job_id = AccountDeletionJobId::from_uuid(Uuid::now_v7());
    let receipt_digest = SecretDigest::from_array([91; 32]);
    let request = AccountDeletionRequest {
        job_id,
        principal_id,
        matrix_user_id: MatrixUserId::new(registration.matrix_user_id.clone())
            .expect("测试 MXID 有效"),
        receipt_digest,
        requested_at: test_time(),
    };
    let created = AccountDeletionRepository::request(&repositories, &request)
        .await
        .expect("删除请求应原子入队");
    assert!(matches!(created, AccountDeletionRequestOutcome::Created(_)));
    let deleting_status: String =
        sqlx::query_scalar("SELECT status FROM agent_room.principal WHERE id = $1")
            .bind(principal_id.as_uuid())
            .fetch_one(&database.runtime)
            .await
            .expect("主体状态应可读");
    assert_eq!(deleting_status, "deleting");

    let lease_expires_at = test_time()
        .checked_add(DurationMillis::new(30_000).expect("租约有效"))
        .expect("租约时间有效");
    let claimed =
        AccountDeletionRepository::claim_due(&repositories, test_time(), lease_expires_at)
            .await
            .expect("应能领取删除任务")
            .expect("应有到期删除任务");
    assert_eq!(claimed.stage, AccountDeletionStage::FederatedDeactivation);
    let local_claim = AccountDeletionRepository::record_federated_deactivation(
        &repositories,
        &claimed,
        test_time(),
    )
    .await
    .expect("Matrix 停用结果应被记录");
    assert_eq!(local_claim.stage, AccountDeletionStage::LocalErasure);
    let completed =
        AccountDeletionRepository::finalize_local(&repositories, &local_claim, test_time())
            .await
            .expect("本地匿名化应完成");
    assert_eq!(completed.stage, AccountDeletionStage::Completed);
    assert_eq!(
        AccountDeletionRepository::find_by_receipt(&repositories, &receipt_digest)
            .await
            .expect("删除回执应可读")
            .expect("删除回执应存在")
            .stage,
        AccountDeletionStage::Completed
    );

    let tombstone: (String, String, String, String) = sqlx::query_as(
        "SELECT status, oidc_issuer, display_name, locale FROM agent_room.principal WHERE id = $1",
    )
    .bind(principal_id.as_uuid())
    .fetch_one(&database.runtime)
    .await
    .expect("匿名化墓碑应可读");
    assert_eq!(
        tombstone,
        (
            "deleted".to_owned(),
            "urn:agent-room:deleted".to_owned(),
            "Deleted account".to_owned(),
            "en".to_owned(),
        )
    );
    assert!(
        AccountDeletionRepository::export(&repositories, principal_id, test_time())
            .await
            .expect("删除后的导出查询应成功")
            .is_none()
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 仓储执行真实读写并拒绝并发覆盖() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    let registration = principal_registration(principal_id, "并发测试主体");

    let created = PrincipalRepository::create(&repositories, &registration)
        .await
        .expect("主体创建应成功");
    assert_eq!(
        PrincipalRepository::find(&repositories, principal_id)
            .await
            .expect("主体读取应成功"),
        Some(created.clone())
    );

    let mut first_writer = created.clone();
    let mut second_writer = created;
    first_writer.suspend().expect("状态转换应成功");
    second_writer.suspend().expect("状态转换应成功");
    let (first_result, second_result) = tokio::join!(
        PrincipalRepository::save(&repositories, &first_writer),
        PrincipalRepository::save(&repositories, &second_writer)
    );
    let outcomes = [first_result, second_result];
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    let conflict = outcomes
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("另一个写入必须失败");
    assert_eq!(conflict.kind(), RepositoryErrorKind::Conflict);

    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let agent_registration = AgentRegistration {
        agent: Agent::register(agent_id),
        owner_id: principal_id,
        matrix_user_id: format!("@agent-{agent_id}:matrix.agent-room.localhost"),
        slug: format!("agent-{agent_id}"),
        display_name: "仓储测试 Agent".to_owned(),
        description: "真实 PostgreSQL 事务测试".to_owned(),
        avatar_content_id: None,
        visibility: AgentVisibility::Private,
        registered_at: test_time(),
    };
    let created_agent = AgentRepository::create(&repositories, &agent_registration)
        .await
        .expect("Agent 与 Owner 必须在同一事务中创建");
    assert_eq!(
        AgentRepository::find(&repositories, agent_id)
            .await
            .expect("Agent 读取应成功"),
        Some(created_agent)
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn agent_创建请求幂等且篡改请求体会冲突() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    PrincipalRepository::create(
        &repositories,
        &principal_registration(principal_id, "Agent Owner"),
    )
    .await
    .expect("Owner 创建应成功");

    let request_id = AgentCreationRequestId::from_uuid(Uuid::now_v7());
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let fingerprint = SecretDigest::from_array([7; 32]);
    let claim = AgentCreationClaim {
        request_id,
        owner_id: principal_id,
        proposed_agent_id: agent_id,
        request_fingerprint: fingerprint,
        reserved_at: test_time(),
    };
    assert_eq!(
        AgentCreationWorkflow::reserve(&repositories, &claim)
            .await
            .expect("首次预留应成功"),
        AgentCreationReservation::Reserved { agent_id }
    );

    let repeated_claim = AgentCreationClaim {
        proposed_agent_id: AgentId::from_uuid(Uuid::now_v7()),
        ..claim.clone()
    };
    assert_eq!(
        AgentCreationWorkflow::reserve(&repositories, &repeated_claim)
            .await
            .expect("重复预留应返回稳定 Agent ID"),
        AgentCreationReservation::Reserved { agent_id }
    );

    let registration = agent_registration(agent_id, principal_id, "idempotent-agent");
    let event = OutboxMessage::new(
        OutboxEventId::from_uuid(Uuid::now_v7()),
        "agent".to_owned(),
        agent_id.as_uuid(),
        "agent.registered.v1".to_owned(),
        serde_json::Map::new(),
        test_time(),
    )
    .expect("注册事件有效");
    AgentCreationWorkflow::complete_with_event(
        &repositories,
        request_id,
        &fingerprint,
        &registration,
        &event,
    )
    .await
    .expect("首次完成应成功");
    AgentCreationWorkflow::complete_with_event(
        &repositories,
        request_id,
        &fingerprint,
        &registration,
        &event,
    )
    .await
    .expect("重复完成不得重复写入");

    let completed = AgentCreationWorkflow::reserve(&repositories, &claim)
        .await
        .expect("完成后重试应读取既有注册");
    assert!(matches!(
        completed,
        AgentCreationReservation::Completed(ref stored) if stored.agent.id() == agent_id
    ));
    let outbox_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_room.outbox_event WHERE aggregate_id = $1")
            .bind(agent_id.as_uuid())
            .fetch_one(&database.runtime)
            .await
            .expect("应能验证 Outbox 幂等性");
    assert_eq!(outbox_count, 1);

    let tampered = AgentCreationClaim {
        request_fingerprint: SecretDigest::from_array([8; 32]),
        ..claim
    };
    let error = AgentCreationWorkflow::reserve(&repositories, &tampered)
        .await
        .expect_err("相同幂等键不得接受不同请求体");
    assert_eq!(error.kind(), RepositoryErrorKind::Conflict);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn agent_成员变更只允许_owner_且不能移除最后一个_owner() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner_id = PrincipalId::from_uuid(Uuid::now_v7());
    let operator_id = PrincipalId::from_uuid(Uuid::now_v7());
    let viewer_id = PrincipalId::from_uuid(Uuid::now_v7());
    let second_owner_id = PrincipalId::from_uuid(Uuid::now_v7());
    for (id, name) in [
        (owner_id, "Owner"),
        (operator_id, "Operator"),
        (viewer_id, "Viewer"),
        (second_owner_id, "Second Owner"),
    ] {
        PrincipalRepository::create(&repositories, &principal_registration(id, name))
            .await
            .expect("成员主体创建应成功");
    }
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    AgentRepository::create(
        &repositories,
        &agent_registration(agent_id, owner_id, "membership-agent"),
    )
    .await
    .expect("Agent 创建应成功");

    let grant_operator =
        membership_change(agent_id, owner_id, operator_id, Some(AgentRole::Operator));
    AgentMembershipTransaction::apply_change(
        &repositories,
        &grant_operator,
        &membership_event(agent_id),
    )
    .await
    .expect("Owner 应能授予 Operator");
    let grant_viewer = membership_change(agent_id, owner_id, viewer_id, Some(AgentRole::Viewer));
    AgentMembershipTransaction::apply_change(
        &repositories,
        &grant_viewer,
        &membership_event(agent_id),
    )
    .await
    .expect("Owner 应能授予 Viewer");

    let unauthorized =
        membership_change(agent_id, viewer_id, second_owner_id, Some(AgentRole::Owner));
    let error = AgentMembershipTransaction::apply_change(
        &repositories,
        &unauthorized,
        &membership_event(agent_id),
    )
    .await
    .expect_err("Viewer 不得转移 Agent 所有权");
    assert_eq!(error.kind(), RepositoryErrorKind::Forbidden);

    let remove_first_owner = membership_change(agent_id, owner_id, owner_id, None);
    let error = AgentMembershipTransaction::apply_change(
        &repositories,
        &remove_first_owner,
        &membership_event(agent_id),
    )
    .await
    .expect_err("不得移除最后一个 Owner");
    assert_eq!(error.kind(), RepositoryErrorKind::Constraint);

    let grant_second_owner =
        membership_change(agent_id, owner_id, second_owner_id, Some(AgentRole::Owner));
    AgentMembershipTransaction::apply_change(
        &repositories,
        &grant_second_owner,
        &membership_event(agent_id),
    )
    .await
    .expect("可先增加第二位 Owner");
    AgentMembershipTransaction::apply_change(
        &repositories,
        &remove_first_owner,
        &membership_event(agent_id),
    )
    .await
    .expect("存在第二位 Owner 后可撤销第一位");

    let memberships = AgentMembershipRepository::find_memberships(&repositories, agent_id)
        .await
        .expect("成员读取应成功")
        .expect("Agent 应存在");
    assert_eq!(memberships.role_of(owner_id), None);
    assert_eq!(memberships.role_of(operator_id), Some(AgentRole::Operator));
    assert_eq!(memberships.role_of(second_owner_id), Some(AgentRole::Owner));

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn agent_实例注册绑定真实设备并拒绝公钥冒用() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let fixture = prepare_instance_fixture(&database.runtime, &repositories, 11).await;
    let owner_id = fixture.owner_id;
    let operator_id = fixture.operator_id;
    let owner_device = fixture.owner_device;
    let agent_id = fixture.agent_id;

    let request_id = AgentInstanceRegistrationRequestId::from_uuid(Uuid::now_v7());
    let fingerprint = SecretDigest::from_array([31; 32]);
    let registration = instance_registration(
        request_id,
        owner_id,
        owner_device,
        agent_id,
        fingerprint,
        [41; 32],
        [51; 32],
    );
    let first = register_instance(&repositories, &registration)
        .await
        .expect("Owner 的已验证设备应能注册实例");

    let repeated = instance_registration(
        request_id,
        owner_id,
        owner_device,
        agent_id,
        fingerprint,
        [41; 32],
        [51; 32],
    );
    let replay = register_instance(&repositories, &repeated)
        .await
        .expect("重复请求必须返回既有实例");
    assert_eq!(replay.binding.id(), first.binding.id());
    assert_eq!(replay.instance.id(), first.instance.id());

    let changed_body = instance_registration(
        request_id,
        owner_id,
        owner_device,
        agent_id,
        SecretDigest::from_array([32; 32]),
        [41; 32],
        [51; 32],
    );
    assert_instance_registration_failure(
        &repositories,
        &changed_body,
        RepositoryErrorKind::Conflict,
        "相同请求 ID 不得篡改请求体",
    )
    .await;

    let stolen_device = instance_registration(
        AgentInstanceRegistrationRequestId::from_uuid(Uuid::now_v7()),
        operator_id,
        owner_device,
        agent_id,
        SecretDigest::from_array([33; 32]),
        [42; 32],
        [52; 32],
    );
    assert_instance_registration_failure(
        &repositories,
        &stolen_device,
        RepositoryErrorKind::Forbidden,
        "Operator 不得借用他人的设备注册实例",
    )
    .await;

    let impersonation = instance_registration(
        AgentInstanceRegistrationRequestId::from_uuid(Uuid::now_v7()),
        owner_id,
        owner_device,
        agent_id,
        SecretDigest::from_array([34; 32]),
        [43; 32],
        [51; 32],
    );
    assert_instance_registration_failure(
        &repositories,
        &impersonation,
        RepositoryErrorKind::Conflict,
        "同一实例公钥不得绑定到另一个适配器主体",
    )
    .await;

    let event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.outbox_event WHERE aggregate_type = 'agent_instance'",
    )
    .fetch_one(&database.runtime)
    .await
    .expect("应能验证实例事件幂等性");
    assert_eq!(event_count, 1);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn agent_实例管理按成员授权并幂等完成本地和_matrix_撤销() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let fixture = prepare_instance_fixture(&database.runtime, &repositories, 13).await;
    let outsider_id = PrincipalId::from_uuid(Uuid::now_v7());
    PrincipalRepository::create(
        &repositories,
        &principal_registration(outsider_id, "Instance Outsider"),
    )
    .await
    .expect("外部主体创建应成功");
    let instance_id =
        register_online_management_instance(&database.runtime, &repositories, &fixture).await;
    assert_instance_management_visibility(&repositories, &fixture, outsider_id).await;
    assert_hidden_instance_revocation(&repositories, outsider_id, instance_id).await;
    assert_local_instance_revocation(
        &database.runtime,
        &repositories,
        fixture.owner_id,
        instance_id,
    )
    .await;

    AgentInstanceMatrixCleanupStore::mark_matrix_device_revoked(
        &repositories,
        instance_id,
        UtcMillis::new(test_time().value() + 1).expect("清理时间有效"),
    )
    .await
    .expect("可记录 Matrix 设备清理完成");
    let completed =
        AgentInstanceManagementRepository::list_for_principal(&repositories, fixture.owner_id)
            .await
            .expect("可读取清理后的状态");
    assert!(completed[0].matrix_device_revoked_at.is_some());

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 产品设备撤销返回待清理_matrix_设备并允许重复请求收敛() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let fixture = prepare_instance_fixture(&database.runtime, &repositories, 14).await;
    let instance_id =
        register_online_management_instance(&database.runtime, &repositories, &fixture).await;
    let before =
        AgentInstanceManagementRepository::list_for_principal(&repositories, fixture.owner_id)
            .await
            .expect("撤销前实例可读取");

    let first = DeviceRevocationTransaction::revoke(
        &repositories,
        fixture.owner_id,
        fixture.owner_device,
        device_security_event(),
    )
    .await
    .expect("产品设备本地撤销成功");
    let DeviceRevocationOutcome::Revoked(pending) = first else {
        panic!("首次撤销必须改变产品设备状态");
    };
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].instance_id, instance_id);
    assert_eq!(pending[0].matrix_user_id, before[0].agent_matrix_user_id);
    assert_eq!(
        pending[0].matrix_device_id,
        before[0].instance.matrix_device_id().as_str()
    );
    assert_product_device_revoked(&repositories, fixture.owner_id, instance_id).await;

    let repeated = DeviceRevocationTransaction::revoke(
        &repositories,
        fixture.owner_id,
        fixture.owner_device,
        device_security_event(),
    )
    .await
    .expect("重复撤销可继续清理");
    assert!(matches!(
        repeated,
        DeviceRevocationOutcome::AlreadyRevoked(ref pending) if pending.len() == 1
    ));
    AgentInstanceMatrixCleanupStore::mark_matrix_device_revoked(
        &repositories,
        instance_id,
        test_time(),
    )
    .await
    .expect("Matrix 清理完成状态可持久化");
    let completed = DeviceRevocationTransaction::revoke(
        &repositories,
        fixture.owner_id,
        fixture.owner_device,
        device_security_event(),
    )
    .await
    .expect("已完成清理的重复撤销保持幂等");
    assert_eq!(
        completed,
        DeviceRevocationOutcome::AlreadyRevoked(Vec::new())
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn agent_实例验签材料保留历史公钥并合并设备失效边界() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let fixture = prepare_instance_fixture(&database.runtime, &repositories, 12).await;
    let registration = instance_registration(
        AgentInstanceRegistrationRequestId::from_uuid(Uuid::now_v7()),
        fixture.owner_id,
        fixture.owner_device,
        fixture.agent_id,
        SecretDigest::from_array([61; 32]),
        [62; 32],
        [63; 32],
    );
    let stored = register_instance(&repositories, &registration)
        .await
        .expect("实例注册成功");

    let active = AgentInstanceVerificationRepository::find_verification_record(
        &repositories,
        stored.instance.id(),
    )
    .await
    .expect("活跃验签材料可读取")
    .expect("实例存在");
    assert_eq!(active.agent_id, fixture.agent_id);
    assert_eq!(active.public_signing_key.as_bytes(), &[63; 32]);
    assert_eq!(active.invalidated_at, None);

    let invalidated_at_ms: i64 = sqlx::query_scalar(
        r"UPDATE agent_room.device
          SET trust_state = 'revoked', revoked_at = clock_timestamp()
          WHERE id = $1
          RETURNING floor(extract(epoch FROM revoked_at) * 1000)::bigint",
    )
    .bind(fixture.owner_device.as_uuid())
    .fetch_one(&database.runtime)
    .await
    .expect("设备可撤销");
    let historical = AgentInstanceVerificationRepository::find_verification_record(
        &repositories,
        stored.instance.id(),
    )
    .await
    .expect("撤销后仍可读取历史公钥")
    .expect("实例历史仍存在");
    assert_eq!(historical.public_signing_key, active.public_signing_key);
    assert_eq!(
        historical.invalidated_at,
        Some(UtcMillis::new(invalidated_at_ms).expect("失效时间有效"))
    );
    assert!(historical.registered_at <= historical.invalidated_at.expect("已有失效时间"));

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 上下文交接授权在同一快照返回两个精确实例及主体角色() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let fixture = prepare_instance_fixture(&database.runtime, &repositories, 70).await;
    let requester = instance_registration(
        AgentInstanceRegistrationRequestId::from_uuid(Uuid::now_v7()),
        fixture.owner_id,
        fixture.owner_device,
        fixture.agent_id,
        SecretDigest::from_array([71; 32]),
        [72; 32],
        [73; 32],
    );
    let requester = register_instance(&repositories, &requester)
        .await
        .expect("请求实例注册成功");

    let target_agent_id = AgentId::from_uuid(Uuid::now_v7());
    AgentRepository::create(
        &repositories,
        &agent_registration(target_agent_id, fixture.owner_id, "handoff-target"),
    )
    .await
    .expect("目标 Agent 创建成功");
    let target = instance_registration(
        AgentInstanceRegistrationRequestId::from_uuid(Uuid::now_v7()),
        fixture.owner_id,
        fixture.owner_device,
        target_agent_id,
        SecretDigest::from_array([74; 32]),
        [75; 32],
        [76; 32],
    );
    let target = register_instance(&repositories, &target)
        .await
        .expect("目标实例注册成功");

    let snapshot = HandoffAccessRepository::inspect_authorization(
        &repositories,
        fixture.owner_id,
        requester.instance.id(),
        target.instance.id(),
    )
    .await
    .expect("交接授权事实可读取")
    .expect("两个实例均存在");
    assert_eq!(snapshot.requester.agent_id, fixture.agent_id);
    assert_eq!(snapshot.requester.role, Some(AgentRole::Owner));
    assert!(snapshot.requester.active);
    assert_eq!(snapshot.target.agent_id, target_agent_id);
    assert_eq!(snapshot.target.role, Some(AgentRole::Owner));
    assert!(snapshot.target.active);

    let outsider = HandoffAccessRepository::find_instance_access(
        &repositories,
        fixture.operator_id,
        target.instance.id(),
    )
    .await
    .expect("无权主体查询仍返回事实")
    .expect("目标实例存在");
    assert_eq!(outsider.role, None);
    assert!(outsider.active);

    sqlx::query(
        "UPDATE agent_room.agent_instance SET status = 'revoked', revoked_at = now() WHERE id = $1",
    )
    .bind(target.instance.id().as_uuid())
    .execute(&database.runtime)
    .await
    .expect("目标实例可撤销");
    let revoked = HandoffAccessRepository::find_instance_access(
        &repositories,
        fixture.owner_id,
        target.instance.id(),
    )
    .await
    .expect("撤销后事实仍可读取")
    .expect("撤销实例仍存在");
    assert!(!revoked.active);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 定向交接覆盖离线排队领取消费拒绝重放过期目标隔离和撤销() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let fixture = prepare_instance_fixture(&database.runtime, &repositories, 77).await;
    let target_registration = instance_registration(
        AgentInstanceRegistrationRequestId::from_uuid(Uuid::now_v7()),
        fixture.owner_id,
        fixture.owner_device,
        fixture.agent_id,
        SecretDigest::from_array([78; 32]),
        [79; 32],
        [80; 32],
    );
    let target = register_instance(&repositories, &target_registration)
        .await
        .expect("离线目标实例注册成功");
    let target_instance_id = target.instance.id();
    let other_target_registration = instance_registration(
        AgentInstanceRegistrationRequestId::from_uuid(Uuid::now_v7()),
        fixture.owner_id,
        fixture.owner_device,
        fixture.agent_id,
        SecretDigest::from_array([84; 32]),
        [85; 32],
        [86; 32],
    );
    let other_target = register_instance(&repositories, &other_target_registration)
        .await
        .expect("隔离对照实例注册成功");
    let other_target_instance_id = other_target.instance.id();
    let handoff_id = queue_targeted_handoff_fixture(
        &database.runtime,
        &repositories,
        &fixture,
        target_instance_id,
    )
    .await;
    assert_targeted_handoff_consumed(&repositories, &fixture, target_instance_id, handoff_id).await;
    assert_target_instance_isolation(
        &database.runtime,
        &repositories,
        &fixture,
        target_instance_id,
        other_target_instance_id,
    )
    .await;
    assert_targeted_handoff_declined(
        &database.runtime,
        &repositories,
        &fixture,
        target_instance_id,
    )
    .await;
    assert_targeted_handoff_expired(
        &database.runtime,
        &repositories,
        &fixture,
        target_instance_id,
    )
    .await;
    assert_target_revocation_fails_pending_handoff(
        &database.runtime,
        &repositories,
        &fixture,
        target_instance_id,
    )
    .await;

    database.close().await;
}

async fn queue_targeted_handoff_fixture(
    pool: &PgPool,
    repositories: &PostgresRepositories,
    fixture: &InstanceFixture,
    target_instance_id: AgentInstanceId,
) -> HandoffId {
    let targets =
        TargetedHandoffRepository::list_targets(repositories, fixture.owner_id, test_time())
            .await
            .expect("可枚举支持交接的目标");
    assert!(
        targets
            .iter()
            .any(|record| record.instance_id == target_instance_id)
    );

    let content_id = ContentId::from_uuid(Uuid::now_v7());
    let room_id = "!targeted-handoff:matrix.test";
    let event_id = "$targeted-handoff:matrix.test";
    seed_targeted_handoff_content(pool, fixture.owner_id, content_id, room_id, event_id).await;
    let handoff_id = HandoffId::from_uuid(Uuid::now_v7());
    let handoff = targeted_handoff(
        handoff_id,
        fixture,
        target_instance_id,
        content_id,
        room_id,
        event_id,
    );
    let fingerprint = TargetedHandoffRequestFingerprint::from_bytes([81; 32]);
    let created = TargetedHandoffRepository::queue(
        repositories,
        QueueTargetedHandoff {
            handoff: &handoff,
            request_fingerprint: fingerprint,
        },
    )
    .await
    .expect("首次排队成功");
    assert!(matches!(created, QueueTargetedHandoffOutcome::Created(_)));
    let replay = TargetedHandoffRepository::queue(
        repositories,
        QueueTargetedHandoff {
            handoff: &handoff,
            request_fingerprint: fingerprint,
        },
    )
    .await
    .expect("同一幂等请求可安全重放");
    assert!(matches!(replay, QueueTargetedHandoffOutcome::Existing(_)));
    let conflict = TargetedHandoffRepository::queue(
        repositories,
        QueueTargetedHandoff {
            handoff: &handoff,
            request_fingerprint: TargetedHandoffRequestFingerprint::from_bytes([82; 32]),
        },
    )
    .await
    .expect_err("同一键不能改写请求");
    assert_eq!(conflict.kind(), RepositoryErrorKind::Conflict);
    handoff_id
}

async fn assert_targeted_handoff_consumed(
    repositories: &PostgresRepositories,
    fixture: &InstanceFixture,
    target_instance_id: AgentInstanceId,
    handoff_id: HandoffId,
) {
    let delivered = TargetedHandoffRepository::claim_next(
        repositories,
        ClaimTargetedHandoff {
            principal_id: fixture.owner_id,
            device_id: fixture.owner_device,
            target_instance_id,
            claimed_at: test_time(),
        },
    )
    .await
    .expect("领取事务成功")
    .expect("存在待领取记录");
    assert_eq!(delivered.status(), TargetedHandoffStatus::Delivered);
    assert_eq!(delivered.fields().id, handoff_id);
    let redelivered = TargetedHandoffRepository::claim_next(
        repositories,
        ClaimTargetedHandoff {
            principal_id: fixture.owner_id,
            device_id: fixture.owner_device,
            target_instance_id,
            claimed_at: test_time(),
        },
    )
    .await
    .expect("未回执交接可以安全重领")
    .expect("已交付记录在终态回执前仍然可见");
    assert_eq!(redelivered.fields().id, handoff_id);
    assert_eq!(redelivered.status(), TargetedHandoffStatus::Delivered);
    assert_eq!(redelivered.delivered_at(), delivered.delivered_at());
    assert_eq!(redelivered.version(), delivered.version());
    let consumed = TargetedHandoffRepository::record_receipt(
        repositories,
        RecordTargetedHandoffReceipt {
            principal_id: fixture.owner_id,
            device_id: fixture.owner_device,
            target_instance_id,
            handoff_id,
            outcome: TargetedHandoffReceiptOutcome::Consumed,
            recorded_at: test_time(),
        },
    )
    .await
    .expect("消费回执事务成功")
    .expect("交接存在");
    assert_eq!(consumed.status(), TargetedHandoffStatus::Consumed);
    let after_receipt = TargetedHandoffRepository::claim_next(
        repositories,
        ClaimTargetedHandoff {
            principal_id: fixture.owner_id,
            device_id: fixture.owner_device,
            target_instance_id,
            claimed_at: test_time(),
        },
    )
    .await
    .expect("终态后查询队列成功");
    assert!(after_receipt.is_none(), "终态交接不得再次投递");
}

async fn assert_target_instance_isolation(
    pool: &PgPool,
    repositories: &PostgresRepositories,
    fixture: &InstanceFixture,
    target_instance_id: AgentInstanceId,
    other_target_instance_id: AgentInstanceId,
) {
    let handoff_id = queue_distinct_targeted_handoff(
        pool,
        repositories,
        fixture,
        target_instance_id,
        "$target-isolation:matrix.test",
        87,
        test_time_after(120_000),
    )
    .await;
    let wrong_target = TargetedHandoffRepository::claim_next(
        repositories,
        ClaimTargetedHandoff {
            principal_id: fixture.owner_id,
            device_id: fixture.owner_device,
            target_instance_id: other_target_instance_id,
            claimed_at: test_time(),
        },
    )
    .await
    .expect("对照实例领取查询成功");
    assert!(wrong_target.is_none(), "非目标实例不得领取交接");
    let still_queued = TargetedHandoffRepository::find_for_principal(
        repositories,
        handoff_id,
        fixture.owner_id,
        test_time(),
    )
    .await
    .expect("隔离验证后可查询交接")
    .expect("目标交接仍存在");
    assert_eq!(still_queued.status(), TargetedHandoffStatus::Queued);
    assert_targeted_handoff_consumed(repositories, fixture, target_instance_id, handoff_id).await;
}

async fn assert_targeted_handoff_declined(
    pool: &PgPool,
    repositories: &PostgresRepositories,
    fixture: &InstanceFixture,
    target_instance_id: AgentInstanceId,
) {
    let handoff_id = queue_distinct_targeted_handoff(
        pool,
        repositories,
        fixture,
        target_instance_id,
        "$target-decline:matrix.test",
        88,
        test_time_after(120_000),
    )
    .await;
    let delivered = TargetedHandoffRepository::claim_next(
        repositories,
        ClaimTargetedHandoff {
            principal_id: fixture.owner_id,
            device_id: fixture.owner_device,
            target_instance_id,
            claimed_at: test_time(),
        },
    )
    .await
    .expect("拒绝场景领取成功")
    .expect("拒绝场景存在待领取记录");
    assert_eq!(delivered.fields().id, handoff_id);
    let decline_code = HandoffFailureCode::new("handoff.user_declined").expect("拒绝码有效");
    let declined = TargetedHandoffRepository::record_receipt(
        repositories,
        RecordTargetedHandoffReceipt {
            principal_id: fixture.owner_id,
            device_id: fixture.owner_device,
            target_instance_id,
            handoff_id,
            outcome: TargetedHandoffReceiptOutcome::Declined(decline_code.clone()),
            recorded_at: test_time(),
        },
    )
    .await
    .expect("拒绝回执事务成功")
    .expect("拒绝交接存在");
    assert_eq!(declined.status(), TargetedHandoffStatus::Declined);
    assert_eq!(declined.failure_code(), Some(&decline_code));
    let replayed = TargetedHandoffRepository::record_receipt(
        repositories,
        RecordTargetedHandoffReceipt {
            principal_id: fixture.owner_id,
            device_id: fixture.owner_device,
            target_instance_id,
            handoff_id,
            outcome: TargetedHandoffReceiptOutcome::Declined(decline_code),
            recorded_at: test_time_after(1),
        },
    )
    .await
    .expect("相同拒绝回执可安全重放")
    .expect("重放时交接仍可审计");
    assert_eq!(replayed.version(), declined.version());
    let conflicting_replay = TargetedHandoffRepository::record_receipt(
        repositories,
        RecordTargetedHandoffReceipt {
            principal_id: fixture.owner_id,
            device_id: fixture.owner_device,
            target_instance_id,
            handoff_id,
            outcome: TargetedHandoffReceiptOutcome::Consumed,
            recorded_at: test_time_after(2),
        },
    )
    .await
    .expect_err("拒绝后不得改写为已消费");
    assert_eq!(conflicting_replay.kind(), RepositoryErrorKind::Conflict);
}

async fn assert_targeted_handoff_expired(
    pool: &PgPool,
    repositories: &PostgresRepositories,
    fixture: &InstanceFixture,
    target_instance_id: AgentInstanceId,
) {
    let expires_at = test_time_after(1_000);
    let handoff_id = queue_distinct_targeted_handoff(
        pool,
        repositories,
        fixture,
        target_instance_id,
        "$target-expiry:matrix.test",
        89,
        expires_at,
    )
    .await;
    let after_expiry = test_time_after(1_001);
    let claimed = TargetedHandoffRepository::claim_next(
        repositories,
        ClaimTargetedHandoff {
            principal_id: fixture.owner_id,
            device_id: fixture.owner_device,
            target_instance_id,
            claimed_at: after_expiry,
        },
    )
    .await
    .expect("到期队列查询成功");
    assert!(claimed.is_none(), "到期交接不得被领取");
    let expired = TargetedHandoffRepository::find_for_principal(
        repositories,
        handoff_id,
        fixture.owner_id,
        after_expiry,
    )
    .await
    .expect("到期交接可查询")
    .expect("到期交接审计记录保留");
    assert_eq!(expired.status(), TargetedHandoffStatus::Expired);
    let late_receipt = TargetedHandoffRepository::record_receipt(
        repositories,
        RecordTargetedHandoffReceipt {
            principal_id: fixture.owner_id,
            device_id: fixture.owner_device,
            target_instance_id,
            handoff_id,
            outcome: TargetedHandoffReceiptOutcome::Consumed,
            recorded_at: after_expiry,
        },
    )
    .await
    .expect_err("到期后不得接受消费回执");
    assert_eq!(late_receipt.kind(), RepositoryErrorKind::Conflict);
}

async fn queue_distinct_targeted_handoff(
    pool: &PgPool,
    repositories: &PostgresRepositories,
    fixture: &InstanceFixture,
    target_instance_id: AgentInstanceId,
    event_id: &str,
    fingerprint_byte: u8,
    expires_at: UtcMillis,
) -> HandoffId {
    let room_id = "!targeted-handoff:matrix.test";
    let content_id = ContentId::from_uuid(Uuid::now_v7());
    seed_targeted_handoff_content(pool, fixture.owner_id, content_id, room_id, event_id).await;
    let handoff_id = HandoffId::from_uuid(Uuid::now_v7());
    let handoff = targeted_handoff_expiring_at(
        handoff_id,
        fixture,
        target_instance_id,
        content_id,
        room_id,
        event_id,
        expires_at,
    );
    let queued = TargetedHandoffRepository::queue(
        repositories,
        QueueTargetedHandoff {
            handoff: &handoff,
            request_fingerprint: TargetedHandoffRequestFingerprint::from_bytes(
                [fingerprint_byte; 32],
            ),
        },
    )
    .await
    .expect("独立交接排队成功");
    assert!(matches!(queued, QueueTargetedHandoffOutcome::Created(_)));
    handoff_id
}

async fn assert_target_revocation_fails_pending_handoff(
    pool: &PgPool,
    repositories: &PostgresRepositories,
    fixture: &InstanceFixture,
    target_instance_id: AgentInstanceId,
) {
    let room_id = "!targeted-handoff:matrix.test";
    let event_id = "$targeted-revoke:matrix.test";
    let second_content_id = ContentId::from_uuid(Uuid::now_v7());
    seed_targeted_handoff_content(pool, fixture.owner_id, second_content_id, room_id, event_id)
        .await;
    let pending_id = HandoffId::from_uuid(Uuid::now_v7());
    let pending = targeted_handoff(
        pending_id,
        fixture,
        target_instance_id,
        second_content_id,
        room_id,
        event_id,
    );
    TargetedHandoffRepository::queue(
        repositories,
        QueueTargetedHandoff {
            handoff: &pending,
            request_fingerprint: TargetedHandoffRequestFingerprint::from_bytes([83; 32]),
        },
    )
    .await
    .expect("第二条交接排队成功");
    AgentInstanceRevocationTransaction::revoke(
        repositories,
        fixture.owner_id,
        target_instance_id,
        &instance_revocation_event(target_instance_id),
    )
    .await
    .expect("目标撤销成功");
    let failed = TargetedHandoffRepository::find_for_principal(
        repositories,
        pending_id,
        fixture.owner_id,
        test_time(),
    )
    .await
    .expect("撤销后的交接可查询")
    .expect("审计记录保留");
    assert_eq!(failed.status(), TargetedHandoffStatus::Failed);
    assert_eq!(
        failed.failure_code().map(HandoffFailureCode::as_str),
        Some("handoff.target_revoked")
    );
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn agent_card_快照可往返且历史被原子裁剪() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner_id = PrincipalId::from_uuid(Uuid::now_v7());
    PrincipalRepository::create(
        &repositories,
        &principal_registration(owner_id, "Agent Card Owner"),
    )
    .await
    .expect("Owner 创建应成功");
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    AgentRepository::create(
        &repositories,
        &agent_registration(agent_id, owner_id, "agent-card-cache"),
    )
    .await
    .expect("Agent 创建应成功");

    let reference_time = test_time().value();
    for index in 0_u8..12 {
        let fetched_at = if index == 0 {
            reference_time - 91 * 24 * 60 * 60 * 1_000
        } else {
            reference_time + i64::from(index)
        };
        let snapshot = agent_card_snapshot(agent_id, index, fetched_at);
        AgentCardSnapshotRepository::save(&repositories, &snapshot)
            .await
            .expect("快照保存应成功");
    }

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.agent_card_snapshot WHERE agent_id = $1",
    )
    .bind(agent_id.as_uuid())
    .fetch_one(&database.runtime)
    .await
    .expect("应能读取快照数量");
    assert_eq!(count, 10);
    let latest = AgentCardSnapshotRepository::find_latest(&repositories, agent_id)
        .await
        .expect("最新快照读取应成功")
        .expect("最新快照应存在");
    assert_eq!(latest.digest().as_bytes(), &[11; 32]);
    assert_eq!(latest.card().name(), "远端测试 Agent");
    let stored: serde_json::Value = sqlx::query_scalar(
        "SELECT normalized_card FROM agent_room.agent_card_snapshot WHERE id = $1",
    )
    .bind(latest.id().as_uuid())
    .fetch_one(&database.runtime)
    .await
    .expect("应能检查规范化持久化内容");
    assert_eq!(stored["schemaVersion"], 1);
    assert!(stored.get("signatures").is_none());

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 事务失败会回滚且外键拒绝孤儿记录() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let missing_owner = PrincipalId::from_uuid(Uuid::now_v7());
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let registration = AgentRegistration {
        agent: Agent::register(agent_id),
        owner_id: missing_owner,
        matrix_user_id: format!("@rollback-{agent_id}:matrix.agent-room.localhost"),
        slug: format!("rollback-{agent_id}"),
        display_name: "回滚测试 Agent".to_owned(),
        description: String::new(),
        avatar_content_id: None,
        visibility: AgentVisibility::Private,
        registered_at: test_time(),
    };

    let error = AgentRepository::create(&repositories, &registration)
        .await
        .expect_err("不存在的 Owner 必须导致事务失败");
    assert_eq!(error.kind(), RepositoryErrorKind::Constraint);
    let persisted: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_room.agent WHERE id = $1")
        .bind(agent_id.as_uuid())
        .fetch_one(&database.runtime)
        .await
        .expect("应能验证回滚结果");
    assert_eq!(persisted, 0);

    let orphan_error = sqlx::query(
        "INSERT INTO agent_room.device (\
            id, principal_id, label, platform, public_signing_key, trust_state, created_at\
         ) VALUES ($1, $2, '孤儿设备', 'windows', $3, 'pending', clock_timestamp())",
    )
    .bind(Uuid::now_v7())
    .bind(Uuid::now_v7())
    .bind(vec![7_u8; 32])
    .execute(&database.runtime)
    .await
    .expect_err("孤儿设备必须被外键拒绝");
    assert_eq!(database_code(&orphan_error).as_deref(), Some("23503"));

    let rolled_back_id = PrincipalId::from_uuid(Uuid::now_v7());
    let mut transaction = database.runtime.begin().await.expect("应能开启显式事务");
    sqlx::query(
        "INSERT INTO agent_room.principal (\
            id, oidc_issuer, oidc_subject, matrix_user_id, display_name, locale,\
            status, created_at, updated_at, version\
         ) VALUES ($1, 'https://issuer.test', $2, $3, '回滚主体', 'zh-CN',\
            'active', clock_timestamp(), clock_timestamp(), 0)",
    )
    .bind(rolled_back_id.as_uuid())
    .bind(format!("subject-{rolled_back_id}"))
    .bind(format!(
        "@rollback-{rolled_back_id}:matrix.agent-room.localhost"
    ))
    .execute(&mut *transaction)
    .await
    .expect("事务内写入应成功");
    transaction.rollback().await.expect("显式回滚应成功");
    let persisted: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_room.principal WHERE id = $1")
            .bind(rolled_back_id.as_uuid())
            .fetch_one(&database.runtime)
            .await
            .expect("应能读取回滚后的状态");
    assert_eq!(persisted, 0);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 审计日志只允许追加写入() {
    let database = TestDatabase::connect().await;
    let event_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO agent_room.audit_event (\
            id, occurred_at, actor_kind, actor_reference, action, target_kind,\
            target_reference, outcome, correlation_id, metadata\
         ) VALUES ($1, clock_timestamp(), 'service', 'integration-test',\
            'database.verify', 'database', 'agent-room', 'allowed', $2, '{}'::jsonb)",
    )
    .bind(event_id)
    .bind(Uuid::now_v7())
    .execute(&database.runtime)
    .await
    .expect("运行时角色应能追加审计事件");

    let permission_error =
        sqlx::query("UPDATE agent_room.audit_event SET outcome = 'failed' WHERE id = $1")
            .bind(event_id)
            .execute(&database.runtime)
            .await
            .expect_err("运行时角色没有审计更新权限");
    assert_eq!(database_code(&permission_error).as_deref(), Some("42501"));

    let trigger_error =
        sqlx::query("UPDATE agent_room.audit_event SET outcome = 'failed' WHERE id = $1")
            .bind(event_id)
            .execute(&database.migration)
            .await
            .expect_err("迁移所有者也不得篡改审计事件");
    assert_eq!(database_code(&trigger_error).as_deref(), Some("55000"));

    database.close().await;
}

fn principal_registration(id: PrincipalId, display_name: &str) -> PrincipalRegistration {
    PrincipalRegistration {
        principal: Principal::new(id),
        oidc_issuer: "https://issuer.test".to_owned(),
        oidc_subject: format!("subject-{id}"),
        matrix_user_id: format!("@principal-{id}:matrix.agent-room.localhost"),
        display_name: display_name.to_owned(),
        avatar_content_id: None,
        locale: "zh-CN".to_owned(),
        registered_at: test_time(),
    }
}

fn agent_registration(agent_id: AgentId, owner_id: PrincipalId, slug: &str) -> AgentRegistration {
    AgentRegistration {
        agent: Agent::register(agent_id),
        owner_id,
        matrix_user_id: format!(
            "@_agent_{}:matrix.agent-room.localhost",
            agent_id.as_uuid().simple()
        ),
        slug: slug.to_owned(),
        display_name: "幂等 Agent".to_owned(),
        description: "真实事务状态机测试".to_owned(),
        avatar_content_id: None,
        visibility: AgentVisibility::Private,
        registered_at: test_time(),
    }
}

fn membership_change(
    agent_id: AgentId,
    actor_id: PrincipalId,
    principal_id: PrincipalId,
    role: Option<AgentRole>,
) -> AgentMembershipChange {
    AgentMembershipChange {
        agent_id,
        actor_id,
        principal_id,
        role,
        changed_at: test_time(),
    }
}

fn membership_event(agent_id: AgentId) -> OutboxMessage {
    OutboxMessage::new(
        OutboxEventId::from_uuid(Uuid::now_v7()),
        "agent".to_owned(),
        agent_id.as_uuid(),
        "agent.membership.changed.v1".to_owned(),
        serde_json::Map::new(),
        test_time(),
    )
    .expect("成员事件有效")
}

struct InstanceFixture {
    owner_id: PrincipalId,
    operator_id: PrincipalId,
    owner_device: DeviceId,
    agent_id: AgentId,
}

async fn register_online_management_instance(
    pool: &PgPool,
    repositories: &PostgresRepositories,
    fixture: &InstanceFixture,
) -> AgentInstanceId {
    let registration = instance_registration(
        AgentInstanceRegistrationRequestId::from_uuid(Uuid::now_v7()),
        fixture.owner_id,
        fixture.owner_device,
        fixture.agent_id,
        SecretDigest::from_array([81; 32]),
        [82; 32],
        [83; 32],
    );
    let stored = register_instance(repositories, &registration)
        .await
        .expect("实例注册成功");
    sqlx::query(
        r"UPDATE agent_room.agent_instance
          SET status = 'online', lease_expires_at = clock_timestamp() + interval '5 minutes'
          WHERE id = $1",
    )
    .bind(stored.instance.id().as_uuid())
    .execute(pool)
    .await
    .expect("测试实例可进入在线状态");
    stored.instance.id()
}

async fn assert_instance_management_visibility(
    repositories: &PostgresRepositories,
    fixture: &InstanceFixture,
    outsider_id: PrincipalId,
) {
    let owner_instances =
        AgentInstanceManagementRepository::list_for_principal(repositories, fixture.owner_id)
            .await
            .expect("Owner 可读取实例");
    assert_eq!(owner_instances.len(), 1);
    assert_eq!(
        owner_instances[0].instance.status(),
        agent_room_domain::agents::AgentInstanceStatus::Online
    );
    let operator_instances =
        AgentInstanceManagementRepository::list_for_principal(repositories, fixture.operator_id)
            .await
            .expect("Operator 可读取实例");
    assert_eq!(operator_instances.len(), 1);
    let outsider_instances =
        AgentInstanceManagementRepository::list_for_principal(repositories, outsider_id)
            .await
            .expect("无权主体查询返回空列表");
    assert!(outsider_instances.is_empty());
}

async fn assert_hidden_instance_revocation(
    repositories: &PostgresRepositories,
    outsider_id: PrincipalId,
    instance_id: AgentInstanceId,
) {
    let hidden = AgentInstanceRevocationTransaction::revoke(
        repositories,
        outsider_id,
        instance_id,
        &instance_revocation_event(instance_id),
    )
    .await
    .expect("无权撤销不会泄漏实例是否存在");
    assert_eq!(hidden, AgentInstanceRevocationOutcome::NotFound);
}

async fn assert_local_instance_revocation(
    pool: &PgPool,
    repositories: &PostgresRepositories,
    owner_id: PrincipalId,
    instance_id: AgentInstanceId,
) {
    let revoked = AgentInstanceRevocationTransaction::revoke(
        repositories,
        owner_id,
        instance_id,
        &instance_revocation_event(instance_id),
    )
    .await
    .expect("Owner 可撤销实例");
    let AgentInstanceRevocationOutcome::Revoked(revoked) = revoked else {
        panic!("首次撤销必须改变状态");
    };
    assert_eq!(
        revoked.instance.status(),
        agent_room_domain::agents::AgentInstanceStatus::Revoked
    );
    assert_eq!(revoked.instance.lease_expires_at(), None);
    assert!(revoked.revoked_at.is_some());
    assert_eq!(revoked.matrix_device_revoked_at, None);

    let repeated = AgentInstanceRevocationTransaction::revoke(
        repositories,
        owner_id,
        instance_id,
        &instance_revocation_event(instance_id),
    )
    .await
    .expect("重复撤销应返回稳定状态");
    assert!(matches!(
        repeated,
        AgentInstanceRevocationOutcome::AlreadyRevoked(ref record)
            if record.instance.id() == instance_id
    ));
    let event_count: i64 = sqlx::query_scalar(
        r"SELECT count(*) FROM agent_room.outbox_event
          WHERE aggregate_id = $1 AND event_type = 'agent.instance.revoked.v1'",
    )
    .bind(instance_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("应能检查撤销事件幂等性");
    assert_eq!(event_count, 1);
}

async fn assert_product_device_revoked(
    repositories: &PostgresRepositories,
    owner_id: PrincipalId,
    instance_id: AgentInstanceId,
) {
    let instances = AgentInstanceManagementRepository::list_for_principal(repositories, owner_id)
        .await
        .expect("设备撤销后的实例可读取");
    let instance = instances
        .iter()
        .find(|record| record.instance.id() == instance_id)
        .expect("关联实例必须保留审计记录");
    assert_eq!(
        instance.instance.status(),
        agent_room_domain::agents::AgentInstanceStatus::Revoked
    );
    assert_eq!(instance.instance.lease_expires_at(), None);
    assert!(instance.revoked_at.is_some());
    assert_eq!(instance.matrix_device_revoked_at, None);
}

fn device_security_event() -> DeviceSecurityEvent {
    DeviceSecurityEvent {
        id: OutboxEventId::from_uuid(Uuid::now_v7()),
        occurred_at: test_time(),
    }
}

async fn prepare_instance_fixture(
    pool: &PgPool,
    repositories: &PostgresRepositories,
    device_key_seed: u8,
) -> InstanceFixture {
    let owner_id = PrincipalId::from_uuid(Uuid::now_v7());
    let operator_id = PrincipalId::from_uuid(Uuid::now_v7());
    for (id, name) in [
        (owner_id, "Instance Owner"),
        (operator_id, "Instance Operator"),
    ] {
        PrincipalRepository::create(repositories, &principal_registration(id, name))
            .await
            .expect("实例成员主体创建应成功");
    }
    let owner_device = DeviceId::from_uuid(Uuid::now_v7());
    insert_verified_device(pool, owner_device, owner_id, device_key_seed).await;
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let slug = format!("instance-agent-{device_key_seed}");
    AgentRepository::create(repositories, &agent_registration(agent_id, owner_id, &slug))
        .await
        .expect("Agent 创建应成功");
    let grant_operator =
        membership_change(agent_id, owner_id, operator_id, Some(AgentRole::Operator));
    AgentMembershipTransaction::apply_change(
        repositories,
        &grant_operator,
        &membership_event(agent_id),
    )
    .await
    .expect("Owner 应能授予 Operator");
    InstanceFixture {
        owner_id,
        operator_id,
        owner_device,
        agent_id,
    }
}

async fn insert_verified_device(
    pool: &PgPool,
    device_id: DeviceId,
    principal_id: PrincipalId,
    key_seed: u8,
) {
    sqlx::query(
        r"INSERT INTO agent_room.device (
            id, principal_id, label, platform, public_signing_key,
            trust_state, verified_at, created_at
        ) VALUES (
            $1, $2, '集成测试设备', 'windows', $3, 'verified',
            to_timestamp($4::double precision / 1000.0),
            to_timestamp($4::double precision / 1000.0)
        )",
    )
    .bind(device_id.as_uuid())
    .bind(principal_id.as_uuid())
    .bind(vec![key_seed; 32])
    .bind(test_time().value())
    .execute(pool)
    .await
    .expect("已验证设备写入应成功");
}

async fn seed_targeted_handoff_content(
    pool: &PgPool,
    principal_id: PrincipalId,
    content_id: ContentId,
    room_id: &str,
    event_id: &str,
) {
    let expires_at = test_time()
        .checked_add(DurationMillis::new(86_400_000).expect("内容期限有效"))
        .expect("内容到期时间有效");
    sqlx::query(
        r"INSERT INTO agent_room.content_object (
               id, owner_principal_id, storage_key, sha256_digest, byte_length,
               media_type, encryption_mode, scan_state, lifecycle_state,
               expires_at, created_at, updated_at
           ) VALUES (
               $1, $2, $3, $4, 256, 'text/markdown', 'server_side', 'clean', 'active',
               to_timestamp($5::double precision / 1000.0),
               to_timestamp($6::double precision / 1000.0),
               to_timestamp($6::double precision / 1000.0)
           )",
    )
    .bind(content_id.as_uuid())
    .bind(principal_id.as_uuid())
    .bind(format!("content/{content_id}/targeted-handoff"))
    .bind(vec![0x42_u8; 32])
    .bind(expires_at.value())
    .bind(test_time().value())
    .execute(pool)
    .await
    .expect("交接内容写入成功");
    sqlx::query(
        r"INSERT INTO agent_room.content_access_policy (
               id, content_id, matrix_room_id, matrix_event_id,
               access_mode, created_at, updated_at
           ) VALUES (
               $1, $2, $3, $4, 'room_member',
               to_timestamp($5::double precision / 1000.0),
               to_timestamp($5::double precision / 1000.0)
           )",
    )
    .bind(Uuid::now_v7())
    .bind(content_id.as_uuid())
    .bind(room_id)
    .bind(event_id)
    .bind(test_time().value())
    .execute(pool)
    .await
    .expect("交接内容策略写入成功");
}

fn targeted_handoff(
    id: HandoffId,
    fixture: &InstanceFixture,
    target_instance_id: AgentInstanceId,
    content_id: ContentId,
    room_id: &str,
    event_id: &str,
) -> TargetedHandoff {
    targeted_handoff_expiring_at(
        id,
        fixture,
        target_instance_id,
        content_id,
        room_id,
        event_id,
        test_time_after(120_000),
    )
}

fn targeted_handoff_expiring_at(
    id: HandoffId,
    fixture: &InstanceFixture,
    target_instance_id: AgentInstanceId,
    content_id: ContentId,
    room_id: &str,
    event_id: &str,
    expires_at: UtcMillis,
) -> TargetedHandoff {
    TargetedHandoff::queue(TargetedHandoffFields {
        id,
        principal_id: fixture.owner_id,
        source_room_id: MatrixRoomReference::new(room_id).expect("房间有效"),
        source_event_id: HandoffSourceEventId::new(event_id).expect("事件有效"),
        source_message_id: MessageId::from_uuid(Uuid::now_v7()),
        target_agent_id: fixture.agent_id,
        target_instance_id,
        content: HandoffContentReference::new(
            content_id,
            Sha256Digest::from_bytes([0x42; 32]),
            ContentByteLength::new(256).expect("内容长度有效"),
            ContentMediaType::new("text/markdown").expect("媒体类型有效"),
        ),
        permissions: HandoffPermissions::new([
            HandoffPermission::ReadText,
            HandoffPermission::IncludeMetadata,
        ])
        .expect("交接权限有效"),
        purpose: HandoffPurpose::Summarize,
        created_at: test_time(),
        expires_at,
    })
    .expect("交接领域对象有效")
}

fn instance_registration(
    request_id: AgentInstanceRegistrationRequestId,
    principal_id: PrincipalId,
    device_id: DeviceId,
    agent_id: AgentId,
    request_fingerprint: SecretDigest,
    subject_hash: [u8; 32],
    signing_key: [u8; 32],
) -> AgentInstanceRegistration {
    let binding_id = AdapterBindingId::from_uuid(Uuid::now_v7());
    let instance_id = AgentInstanceId::from_uuid(Uuid::now_v7());
    let binding = AdapterBinding::register(
        binding_id,
        agent_id,
        "codex".to_owned(),
        Some(AdapterSubjectHash::new(subject_hash.to_vec()).expect("主体摘要有效")),
        "1.0".to_owned(),
    )
    .expect("适配器绑定有效");
    let instance = AgentInstance::register(
        instance_id,
        agent_id,
        device_id,
        binding_id,
        AgentInstancePublicSigningKey::new(signing_key.to_vec()).expect("实例签名公钥有效"),
        AgentMatrixDeviceId::new(format!("AR_{}", instance_id.as_uuid().simple()))
            .expect("Matrix Device ID 有效"),
    );
    let mut configuration = serde_json::Map::new();
    configuration.insert(
        "mode".to_owned(),
        serde_json::Value::String("observe".to_owned()),
    );
    configuration.insert(
        "capabilities".to_owned(),
        serde_json::json!(["targeted_handoff_v1"]),
    );
    AgentInstanceRegistration {
        request_id,
        principal_id,
        device_id,
        request_fingerprint,
        binding,
        binding_configuration: configuration,
        instance,
        registered_at: test_time(),
    }
}

fn instance_event(registration: &AgentInstanceRegistration) -> OutboxMessage {
    OutboxMessage::new(
        OutboxEventId::from_uuid(Uuid::now_v7()),
        "agent_instance".to_owned(),
        registration.instance.id().as_uuid(),
        "agent.instance.registered.v1".to_owned(),
        serde_json::Map::new(),
        test_time(),
    )
    .expect("实例注册事件有效")
}

fn instance_revocation_event(instance_id: AgentInstanceId) -> OutboxMessage {
    OutboxMessage::new(
        OutboxEventId::from_uuid(Uuid::now_v7()),
        "agent_instance".to_owned(),
        instance_id.as_uuid(),
        "agent.instance.revoked.v1".to_owned(),
        serde_json::Map::new(),
        test_time(),
    )
    .expect("实例撤销事件有效")
}

async fn register_instance(
    repositories: &PostgresRepositories,
    registration: &AgentInstanceRegistration,
) -> agent_room_application::persistence::RepositoryResult<StoredAgentInstanceRegistration> {
    AgentInstanceRegistrationTransaction::register_with_event(
        repositories,
        registration,
        &instance_event(registration),
    )
    .await
}

async fn assert_instance_registration_failure(
    repositories: &PostgresRepositories,
    registration: &AgentInstanceRegistration,
    expected_kind: RepositoryErrorKind,
    reason: &str,
) {
    let error = register_instance(repositories, registration)
        .await
        .expect_err(reason);
    assert_eq!(error.kind(), expected_kind);
}

fn test_time() -> UtcMillis {
    UtcMillis::new(1_700_000_000_000).expect("测试时间戳必须有效")
}

fn test_time_after(milliseconds: u64) -> UtcMillis {
    test_time()
        .checked_add(DurationMillis::new(milliseconds).expect("测试时间偏移有效"))
        .expect("偏移后的测试时间有效")
}

fn agent_card_snapshot(agent_id: AgentId, seed: u8, fetched_at: i64) -> AgentCardSnapshot {
    let fetched_at = UtcMillis::new(fetched_at).expect("测试抓取时间有效");
    let expires_at = UtcMillis::new(fetched_at.value() + 60_000).expect("测试过期时间有效");
    let capabilities = AgentCardCapabilities::new(true, false, false, Vec::new(), &BTreeSet::new())
        .expect("测试能力有效");
    let card = NormalizedAgentCard::new(NormalizedAgentCardFields {
        name: "远端测试 Agent".to_owned(),
        description: "仅保存安全规范化资料".to_owned(),
        provider: None,
        version: "1.0.0".to_owned(),
        endpoints: vec![
            AgentCardEndpoint::new(
                "https://agent.example/a2a".to_owned(),
                AgentCardTransport::JsonRpc,
                AgentCardProtocolVersion::V1_0,
                None,
                AgentEndpointVerificationState::Verified,
            )
            .expect("测试端点有效"),
        ],
        capabilities,
        security_schemes: Vec::new(),
        default_input_modes: vec!["text/plain".to_owned()],
        default_output_modes: vec!["text/plain".to_owned()],
        skills: vec![
            AgentCardSkill::new(
                "chat".to_owned(),
                "聊天".to_owned(),
                "公开能力".to_owned(),
                vec!["chat".to_owned()],
                Vec::new(),
                Vec::new(),
            )
            .expect("测试技能有效"),
        ],
    })
    .expect("测试资料有效");
    AgentCardSnapshot::new(AgentCardSnapshotFields {
        id: AgentCardSnapshotId::from_uuid(Uuid::now_v7()),
        agent_id,
        source_url: AgentCardSourceUrl::new(
            "https://agent.example/.well-known/agent-card.json".to_owned(),
        )
        .expect("测试来源有效"),
        digest: AgentCardDigest::from_array([seed; 32]),
        card,
        verification: AgentCardVerificationState::Unverified,
        fetched_at,
        expires_at,
    })
    .expect("测试快照有效")
}

fn required_url(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("缺少真实数据库测试配置 {name}"))
}

async fn connect_pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .min_connections(0)
        .max_connections(5)
        .connect(url)
        .await
        .expect("真实 PostgreSQL 必须可连接")
}

fn database_code(error: &sqlx::Error) -> Option<String> {
    match error {
        sqlx::Error::Database(database_error) => database_error.code().map(Cow::into_owned),
        _ => None,
    }
}
