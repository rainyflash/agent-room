use super::*;
use agent_room_application::reception::{
    ReceptionCommand, ReceptionPending, ReceptionProgress, ReceptionRepository, ReceptionRequest,
    ReceptionStatus,
};
use agent_room_domain::{agents::host_agent_slug, ids::AgentCreationRequestId};

async fn setup(pool: &PgPool) -> (AutomationFixture, ReceptionRequest) {
    let fixture = seed_automation_fixture(pool).await;
    let session_key = Uuid::now_v7();
    sqlx::query("UPDATE agent_room.agent SET slug=$2 WHERE id=$1")
        .bind(fixture.agent.as_uuid())
        .bind(host_agent_slug(AgentCreationRequestId::from_uuid(
            session_key,
        )))
        .execute(pool)
        .await
        .expect("set stable host identity");
    let request = ReceptionRequest {
        agent_id: fixture.agent.as_uuid(),
        instance_id: fixture.instance.as_uuid(),
        catalog_id: fixture.catalog.as_uuid(),
        room_id: fixture.matrix_room.as_str().into(),
        run_id: Uuid::now_v7(),
        command: ReceptionCommand::Claim {
            session_key,
            display_name: "Reception".into(),
            initial: ReceptionProgress::default(),
        },
    };
    (fixture, request)
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn registered_host_identity_accepts_its_session_and_rejects_another() {
    let database = TestDatabase::connect().await;
    let (fixture, request) = setup(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let wrong_session = ReceptionRequest {
        command: ReceptionCommand::Claim {
            session_key: Uuid::now_v7(),
            display_name: "Reception".into(),
            initial: ReceptionProgress::default(),
        },
        ..request.clone()
    };
    assert_eq!(
        repositories
            .execute(fixture.principal, fixture.device, &wrong_session)
            .await
            .expect_err("an owned Agent still requires its original host session")
            .kind(),
        RepositoryErrorKind::Forbidden
    );
    let record = repositories
        .execute(fixture.principal, fixture.device, &request)
        .await
        .expect("the compact identity created during host registration must be accepted");
    assert_eq!(record.status, ReceptionStatus::Active);
    assert_eq!(record.agent_id, request.agent_id);
    assert_eq!(record.instance_id, request.instance_id);
    assert_eq!(record.run_id, request.run_id);
    database.close().await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn authenticated_reception_starts_connecting_instances_and_renews_expired_activity() {
    let database = TestDatabase::connect().await;
    let (fixture, request) = setup(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    sqlx::query("UPDATE agent_room.agent_instance SET status='connecting', last_seen_at=NULL, lease_expires_at=NULL WHERE id=$1")
        .bind(fixture.instance.as_uuid()).execute(&database.runtime).await.expect("match a freshly registered instance");
    assert_eq!(
        repositories
            .execute(fixture.principal, fixture.device, &request)
            .await
            .expect("a signed request from the registered device is its first activity")
            .status,
        ReceptionStatus::Active
    );
    sqlx::query("UPDATE agent_room.agent_instance SET status='offline', last_seen_at=clock_timestamp()-interval '10 minutes', lease_expires_at=clock_timestamp()-interval '5 minutes' WHERE id=$1")
        .bind(fixture.instance.as_uuid()).execute(&database.runtime).await.expect("expire activity during a connection gap");
    let heartbeat = ReceptionRequest {
        command: ReceptionCommand::Heartbeat,
        ..request
    };
    assert_eq!(
        repositories
            .execute(fixture.principal, fixture.device, &heartbeat)
            .await
            .expect("the authenticated owner can resume its existing execution")
            .status,
        ReceptionStatus::Active
    );
    let online: bool = sqlx::query_scalar("SELECT status='online' AND last_seen_at IS NOT NULL AND lease_expires_at > clock_timestamp() FROM agent_room.agent_instance WHERE id=$1")
        .bind(fixture.instance.as_uuid()).fetch_one(&database.runtime).await.expect("read renewed activity");
    assert!(online);
    database.close().await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn reception_cannot_reactivate_an_instance_after_device_revocation() {
    let database = TestDatabase::connect().await;
    let (fixture, request) = setup(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    sqlx::query("UPDATE agent_room.agent_instance SET status='connecting', last_seen_at=NULL, lease_expires_at=NULL WHERE id=$1")
        .bind(fixture.instance.as_uuid()).execute(&database.runtime).await.expect("new instance");
    sqlx::query("UPDATE agent_room.device SET trust_state='revoked', revoked_at=clock_timestamp() WHERE id=$1")
        .bind(fixture.device.as_uuid()).execute(&database.runtime).await.expect("revoke device");
    assert_eq!(
        repositories
            .execute(fixture.principal, fixture.device, &request)
            .await
            .expect_err("revoked device cannot establish a reception lease")
            .kind(),
        RepositoryErrorKind::Forbidden
    );
    let unchanged: bool = sqlx::query_scalar("SELECT status='connecting' AND last_seen_at IS NULL AND lease_expires_at IS NULL FROM agent_room.agent_instance WHERE id=$1")
        .bind(fixture.instance.as_uuid()).fetch_one(&database.runtime).await.expect("read rejected activity");
    assert!(unchanged);
    database.close().await;
}

async fn second_device(pool: &PgPool, fixture: &AutomationFixture) -> (DeviceId, AgentInstanceId) {
    let device = DeviceId::from_uuid(Uuid::now_v7());
    let instance = AgentInstanceId::from_uuid(Uuid::now_v7());
    sqlx::query("INSERT INTO agent_room.device (id,principal_id,label,platform,public_signing_key,matrix_device_id,trust_state,last_seen_at,created_at,verified_at) VALUES($1,$2,'Second computer','windows',$3,'SECOND','verified',statement_timestamp(),statement_timestamp(),statement_timestamp())")
        .bind(device.as_uuid()).bind(fixture.principal.as_uuid()).bind(signing_key(device.as_uuid()))
        .execute(pool).await.expect("create second device");
    sqlx::query("INSERT INTO agent_room.agent_instance (id,agent_id,device_id,adapter_binding_id,public_signing_key,matrix_device_id,status,lease_expires_at,last_seen_at,created_at) SELECT $1,agent_id,$2,adapter_binding_id,$3,'SECOND-AGENT','online',clock_timestamp()+interval '5 minutes',clock_timestamp(),clock_timestamp() FROM agent_room.agent_instance WHERE id=$4")
        .bind(instance.as_uuid()).bind(device.as_uuid()).bind(signing_key(instance.as_uuid())).bind(fixture.instance.as_uuid())
        .execute(pool).await.expect("create second instance of same Agent");
    (device, instance)
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn concurrent_claims_have_one_owner_and_release_fences_old_execution() {
    let database = TestDatabase::connect().await;
    let (fixture, first) = setup(&database.runtime).await;
    let (device, instance) = second_device(&database.runtime, &fixture).await;
    let second = ReceptionRequest {
        instance_id: instance.as_uuid(),
        run_id: Uuid::now_v7(),
        ..first.clone()
    };
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let (a, b) = tokio::join!(
        repositories.execute(fixture.principal, fixture.device, &first),
        repositories.execute(fixture.principal, device, &second),
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let (winner, owner, loser, next) = if a.is_ok() {
        (first, fixture.device, second, device)
    } else {
        (second, device, first, fixture.device)
    };
    let draining = repositories
        .drain(
            fixture.principal,
            fixture.agent,
            fixture.catalog,
            Some(next),
        )
        .await
        .expect("request transfer");
    assert_eq!(draining.status, ReceptionStatus::Draining);
    assert!(
        repositories
            .execute(fixture.principal, next, &loser)
            .await
            .is_err()
    );
    let release = ReceptionRequest {
        command: ReceptionCommand::Release,
        ..winner.clone()
    };
    assert_eq!(
        repositories
            .execute(fixture.principal, owner, &release)
            .await
            .expect("release")
            .status,
        ReceptionStatus::Idle
    );
    let transferred = repositories
        .execute(fixture.principal, next, &loser)
        .await
        .expect("claim after acknowledgement");
    assert_eq!(transferred.instance_id, loser.instance_id);
    assert!(
        repositories
            .execute(fixture.principal, owner, &release)
            .await
            .is_err(),
        "late release must not remove new owner"
    );
    assert!(
        repositories
            .execute(fixture.principal, owner, &winner)
            .await
            .is_err(),
        "old run cannot restart"
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn transferred_pending_reply_keeps_submission_and_consumption() {
    let database = TestDatabase::connect().await;
    let (fixture, first) = setup(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    repositories
        .execute(fixture.principal, fixture.device, &first)
        .await
        .expect("claim");
    let now = database_now(&database.runtime).await;
    let authorization = grant(&fixture, now, 1, Some(1), 60_000);
    AutomationGrantRepository::create(&repositories, &authorization)
        .await
        .expect("grant");
    let mut send = consumption(&fixture, authorization.id(), now, submission_id());
    send.reception_run_id = Some(first.run_id);
    let progress = ReceptionProgress {
        after_event_id: Some("$previous".into()),
        pending: Some(ReceptionPending {
            event_id: "$incoming".into(),
            message_id: Uuid::now_v7(),
            submission_id: send.submission_id.as_uuid(),
        }),
        retry: None,
    };
    save_and_verify_replay(&repositories, &fixture, &first, &progress, &send).await;
    let (device, instance) = second_device(&database.runtime, &fixture).await;
    repositories
        .drain(
            fixture.principal,
            fixture.agent,
            fixture.catalog,
            Some(device),
        )
        .await
        .expect("drain");
    assert_eq!(
        AutomationGrantRepository::consume(&repositories, &send)
            .await
            .expect_err("draining owner cannot send again")
            .kind(),
        RepositoryErrorKind::Forbidden
    );
    repositories
        .execute(
            fixture.principal,
            fixture.device,
            &ReceptionRequest {
                command: ReceptionCommand::Release,
                ..first.clone()
            },
        )
        .await
        .expect("release");
    let second = ReceptionRequest {
        instance_id: instance.as_uuid(),
        run_id: Uuid::now_v7(),
        ..first
    };
    let restored = repositories
        .execute(fixture.principal, device, &second)
        .await
        .expect("new owner");
    assert_eq!(restored.progress, progress);
    let second_fixture = AutomationFixture {
        device,
        instance,
        ..fixture
    };
    let next_grant = grant(&second_fixture, now, 1, Some(1), 60_000);
    AutomationGrantRepository::create(&repositories, &next_grant)
        .await
        .expect("explicitly authorize new device");
    let mut retried = consumption(&second_fixture, next_grant.id(), now, send.submission_id);
    retried.reception_run_id = Some(second.run_id);
    assert!(matches!(
        AutomationGrantRepository::consume(&repositories, &retried)
            .await
            .expect("retry same pending submission"),
        AutomationConsumptionOutcome::Consumed { reused: true, .. }
    ));
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.automation_consumption WHERE submission_id=$1",
    )
    .bind(send.submission_id.as_uuid())
    .fetch_one(&database.runtime)
    .await
    .expect("count");
    assert_eq!(count, 1);
    retried.submission_id = submission_id();
    assert_eq!(
        AutomationGrantRepository::consume(&repositories, &retried)
            .await
            .expect_err("undeclared submission rejected")
            .kind(),
        RepositoryErrorKind::Forbidden
    );
    assert_unused(&repositories, next_grant.id(), now).await;
    database.close().await;
}

async fn assert_unused(repositories: &PostgresRepositories, id: AutomationGrantId, now: UtcMillis) {
    let current = AutomationGrantRepository::find(repositories, id, now)
        .await
        .expect("find")
        .expect("new grant");
    assert_eq!(
        current.usage.total_messages, 0,
        "replay never charges another grant"
    );
}

async fn save_and_verify_replay(
    repositories: &PostgresRepositories,
    fixture: &AutomationFixture,
    first: &ReceptionRequest,
    progress: &ReceptionProgress,
    send: &AutomationConsumptionRequest,
) {
    let saved = ReceptionRequest {
        command: ReceptionCommand::Save {
            revision: 0,
            progress: progress.clone(),
        },
        ..first.clone()
    };
    repositories
        .execute(fixture.principal, fixture.device, &saved)
        .await
        .expect("save before model starts");
    for expected_reuse in [false, true] {
        let AutomationConsumptionOutcome::Consumed { reused, .. } =
            AutomationGrantRepository::consume(repositories, send)
                .await
                .expect("consume or replay exhausted grant")
        else {
            panic!("pending submission should be permitted");
        };
        assert_eq!(reused, expected_reuse);
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn offline_presence_does_not_transfer_until_matrix_device_revoked() {
    let database = TestDatabase::connect().await;
    let (fixture, first) = setup(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    repositories
        .execute(fixture.principal, fixture.device, &first)
        .await
        .expect("claim");
    let stranger = seed_automation_fixture(&database.runtime).await;
    assert!(
        repositories
            .drain(stranger.principal, fixture.agent, fixture.catalog, None)
            .await
            .is_err()
    );
    assert!(
        repositories
            .list(stranger.principal)
            .await
            .expect("private list")
            .is_empty()
    );
    sqlx::query(
        "UPDATE agent_room.agent_instance SET status='offline', lease_expires_at=NULL WHERE id=$1",
    )
    .bind(fixture.instance.as_uuid())
    .execute(&database.runtime)
    .await
    .expect("offline");
    assert_eq!(
        repositories
            .drain(fixture.principal, fixture.agent, fixture.catalog, None)
            .await
            .expect("request manual takeover")
            .status,
        ReceptionStatus::Draining
    );
    sqlx::query("UPDATE agent_room.agent_instance SET status='revoked', revoked_at=clock_timestamp(), matrix_device_revoked_at=clock_timestamp() WHERE id=$1")
        .bind(fixture.instance.as_uuid()).execute(&database.runtime).await.expect("complete device revocation");
    assert_eq!(
        repositories
            .drain(fixture.principal, fixture.agent, fixture.catalog, None)
            .await
            .expect("recover stopped instance")
            .status,
        ReceptionStatus::Idle
    );
    assert!(
        repositories
            .execute(fixture.principal, fixture.device, &first)
            .await
            .is_err()
    );
    database.close().await;
}
