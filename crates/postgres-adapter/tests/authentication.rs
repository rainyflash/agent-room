use std::env;

use agent_room_application::ports::{
    DesktopAuthorizationCodeRegistration, DesktopLoginCompletionTransaction,
    DesktopSessionExchangeTransaction, DesktopSessionRegistration, LoginAttempt, LoginAttemptStore,
    LoginCompletionTransaction, LoginDelivery, PkceCodeChallenge, PrincipalRegistration,
    PrincipalSuspensionTransaction, ProfileImportConsent, SafeReturnPath, SecretDigest,
    SecretValue, WebSessionRegistration, WebSessionStore,
};
use agent_room_domain::{
    identity::{Principal, PrincipalStatus},
    ids::{LoginAttemptId, PrincipalId, WebSessionId},
    time::UtcMillis,
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
async fn 登录尝试严格绑定浏览器与状态并且只能消费一次() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let attempt = login_attempt(1, 2, test_time(0), test_time(600_000));
    LoginAttemptStore::create(&repositories, &attempt)
        .await
        .expect("登录尝试应写入");

    let wrong_state = SecretDigest::from_array([9; 32]);
    assert!(
        LoginAttemptStore::consume(
            &repositories,
            &attempt.browser_secret_digest,
            &wrong_state,
            test_time(1_000),
        )
        .await
        .expect("错误状态只返回空结果")
        .is_none()
    );
    let consumed = LoginAttemptStore::consume(
        &repositories,
        &attempt.browser_secret_digest,
        &attempt.state_digest,
        test_time(1_000),
    )
    .await
    .expect("正确状态可消费")
    .expect("登录尝试存在");
    assert_eq!(consumed.id, attempt.id);
    assert!(
        LoginAttemptStore::consume(
            &repositories,
            &attempt.browser_secret_digest,
            &attempt.state_digest,
            test_time(2_000),
        )
        .await
        .expect("重放只返回空结果")
        .is_none()
    );

    let expired = login_attempt(3, 4, test_time(0), test_time(10_000));
    LoginAttemptStore::create(&repositories, &expired)
        .await
        .expect("过期前可写入");
    assert!(
        LoginAttemptStore::consume(
            &repositories,
            &expired.browser_secret_digest,
            &expired.state_digest,
            test_time(10_000),
        )
        .await
        .expect("到期尝试只返回空结果")
        .is_none()
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 并发首次登录只建立一个主体但分别建立会话() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let first_principal = principal_registration(
        PrincipalId::from_uuid(Uuid::now_v7()),
        "https://issuer.example/realms/agent-room",
        "same-subject",
        "首次资料",
    );
    let second_principal = principal_registration(
        PrincipalId::from_uuid(Uuid::now_v7()),
        "https://issuer.example/realms/agent-room",
        "same-subject",
        "竞争资料",
    );
    let first_session = session_registration(WebSessionId::from_uuid(Uuid::now_v7()), 11);
    let second_session = session_registration(WebSessionId::from_uuid(Uuid::now_v7()), 12);

    let (first, second) = tokio::join!(
        LoginCompletionTransaction::complete(&repositories, &first_principal, &first_session),
        LoginCompletionTransaction::complete(&repositories, &second_principal, &second_session)
    );
    let first = first.expect("第一个登录应成功");
    let second = second.expect("并发重复主体应幂等成功");
    assert_eq!(first.account.principal.id(), second.account.principal.id());
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.principal WHERE oidc_issuer = $1 AND oidc_subject = $2",
    )
    .bind("https://issuer.example/realms/agent-room")
    .bind("same-subject")
    .fetch_one(&database.runtime)
    .await
    .expect("可统计主体");
    assert_eq!(count, 1);

    assert!(
        WebSessionStore::find_active(
            &repositories,
            &first_session.secret_digest,
            test_time(1_000),
        )
        .await
        .expect("首个会话可查")
        .is_some()
    );
    assert!(
        WebSessionStore::find_active(
            &repositories,
            &second_session.secret_digest,
            test_time(1_000),
        )
        .await
        .expect("第二个会话可查")
        .is_some()
    );

    WebSessionStore::revoke(
        &repositories,
        &first_session.secret_digest,
        test_time(2_000),
    )
    .await
    .expect("登出应成功");
    assert!(
        WebSessionStore::find_active(
            &repositories,
            &first_session.secret_digest,
            test_time(3_000),
        )
        .await
        .expect("撤销会话可查询")
        .is_none()
    );

    let suspended = PrincipalSuspensionTransaction::suspend(
        &repositories,
        second.account.principal.id(),
        test_time(4_000),
    )
    .await
    .expect("主体暂停应成功");
    assert_eq!(suspended.status(), PrincipalStatus::Suspended);
    let repeated = PrincipalSuspensionTransaction::suspend(
        &repositories,
        second.account.principal.id(),
        test_time(5_000),
    )
    .await
    .expect("重复暂停应幂等");
    assert_eq!(repeated.version(), suspended.version());
    assert!(
        WebSessionStore::find_active(
            &repositories,
            &second_session.secret_digest,
            test_time(6_000),
        )
        .await
        .expect("暂停后会话可查询")
        .is_none()
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 会话写入冲突会回滚首次主体投影() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let session_id = WebSessionId::from_uuid(Uuid::now_v7());
    let existing_principal = principal_registration(
        PrincipalId::from_uuid(Uuid::now_v7()),
        "https://issuer.example",
        "existing-subject",
        "已有主体",
    );
    let existing_session = session_registration(session_id, 21);
    LoginCompletionTransaction::complete(&repositories, &existing_principal, &existing_session)
        .await
        .expect("首个会话应建立");

    let rolled_back_id = PrincipalId::from_uuid(Uuid::now_v7());
    let rolled_back_principal = principal_registration(
        rolled_back_id,
        "https://issuer.example",
        "rollback-subject",
        "应回滚主体",
    );
    let conflicting_session = session_registration(session_id, 22);
    LoginCompletionTransaction::complete(
        &repositories,
        &rolled_back_principal,
        &conflicting_session,
    )
    .await
    .expect_err("重复会话主键必须让整个事务失败");

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_room.principal WHERE id = $1")
        .bind(rolled_back_id.as_uuid())
        .fetch_one(&database.runtime)
        .await
        .expect("可检查回滚主体");
    assert_eq!(count, 0);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 桌面授权码交换原子校验_pkce_过期和重放() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let principal = principal_registration(
        PrincipalId::from_uuid(Uuid::now_v7()),
        "https://issuer.example",
        "desktop-subject",
        "桌面用户",
    );
    let authorization = DesktopAuthorizationCodeRegistration {
        code_digest: SecretDigest::from_array([31; 32]),
        code_challenge: PkceCodeChallenge::new("c".repeat(43)).expect("challenge 有效"),
        authenticated_at: test_time(0),
        created_at: test_time(0),
        expires_at: test_time(600_000),
    };
    DesktopLoginCompletionTransaction::complete_desktop(&repositories, &principal, &authorization)
        .await
        .expect("桌面授权码应与主体原子写入");
    let session = DesktopSessionRegistration {
        id: WebSessionId::from_uuid(Uuid::now_v7()),
        secret_digest: SecretDigest::from_array([32; 32]),
        created_at: test_time(1_000),
        expires_at: test_time(28_801_000),
    };
    assert!(
        DesktopSessionExchangeTransaction::exchange_desktop(
            &repositories,
            &authorization.code_digest,
            &PkceCodeChallenge::new("x".repeat(43)).expect("错误 challenge 仍满足格式"),
            &session,
            test_time(1_000),
        )
        .await
        .expect("错误 PKCE 返回空结果")
        .is_none()
    );
    let exchanged = DesktopSessionExchangeTransaction::exchange_desktop(
        &repositories,
        &authorization.code_digest,
        &authorization.code_challenge,
        &session,
        test_time(1_000),
    )
    .await
    .expect("正确 PKCE 可交换")
    .expect("授权码尚未消费");
    assert_eq!(exchanged.authenticated_at, authorization.authenticated_at);
    assert!(
        DesktopSessionExchangeTransaction::exchange_desktop(
            &repositories,
            &authorization.code_digest,
            &authorization.code_challenge,
            &DesktopSessionRegistration {
                id: WebSessionId::from_uuid(Uuid::now_v7()),
                secret_digest: SecretDigest::from_array([33; 32]),
                created_at: test_time(2_000),
                expires_at: test_time(28_802_000),
            },
            test_time(2_000),
        )
        .await
        .expect("重放返回空结果")
        .is_none()
    );

    let expired_authorization = DesktopAuthorizationCodeRegistration {
        code_digest: SecretDigest::from_array([34; 32]),
        code_challenge: PkceCodeChallenge::new("e".repeat(43)).expect("challenge 有效"),
        authenticated_at: test_time(0),
        created_at: test_time(0),
        expires_at: test_time(10_000),
    };
    DesktopLoginCompletionTransaction::complete_desktop(
        &repositories,
        &principal,
        &expired_authorization,
    )
    .await
    .expect("过期前可写入授权码");
    assert!(
        DesktopSessionExchangeTransaction::exchange_desktop(
            &repositories,
            &expired_authorization.code_digest,
            &expired_authorization.code_challenge,
            &DesktopSessionRegistration {
                id: WebSessionId::from_uuid(Uuid::now_v7()),
                secret_digest: SecretDigest::from_array([35; 32]),
                created_at: test_time(10_000),
                expires_at: test_time(28_810_000),
            },
            test_time(10_000),
        )
        .await
        .expect("到期授权码返回空结果")
        .is_none()
    );

    database.close().await;
}

fn login_attempt(
    browser_byte: u8,
    state_byte: u8,
    created_at: UtcMillis,
    expires_at: UtcMillis,
) -> LoginAttempt {
    LoginAttempt {
        id: LoginAttemptId::from_uuid(Uuid::now_v7()),
        browser_secret_digest: SecretDigest::from_array([browser_byte; 32]),
        state_digest: SecretDigest::from_array([state_byte; 32]),
        nonce: SecretValue::new("n".repeat(32)).expect("nonce 有效"),
        pkce_verifier: SecretValue::new("v".repeat(43)).expect("PKCE 有效"),
        delivery: LoginDelivery::Web {
            return_path: SafeReturnPath::new("/rooms/lobby").expect("回跳路径有效"),
        },
        profile_import: ProfileImportConsent::default(),
        created_at,
        expires_at,
    }
}

fn principal_registration(
    id: PrincipalId,
    issuer: &str,
    subject: &str,
    display_name: &str,
) -> PrincipalRegistration {
    PrincipalRegistration {
        principal: Principal::new(id),
        oidc_issuer: issuer.to_owned(),
        oidc_subject: subject.to_owned(),
        matrix_user_id: format!("@user-{id}:matrix.example"),
        display_name: display_name.to_owned(),
        avatar_content_id: None,
        locale: "zh-CN".to_owned(),
        registered_at: test_time(0),
    }
}

fn session_registration(id: WebSessionId, secret_byte: u8) -> WebSessionRegistration {
    WebSessionRegistration {
        id,
        secret_digest: SecretDigest::from_array([secret_byte; 32]),
        authenticated_at: test_time(0),
        created_at: test_time(0),
        expires_at: test_time(28_800_000),
    }
}

fn test_time(offset: i64) -> UtcMillis {
    UtcMillis::new(1_700_000_000_000 + offset).expect("测试时间有效")
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
