use std::sync::{Arc, Mutex};

use agent_room_domain::{
    ids::{PrincipalId, WebSessionId},
    time::UtcMillis,
};
use uuid::Uuid;

use super::{
    SignOutElsewhereDependencies, SignOutElsewhereFailure, SignOutElsewhereService,
    SignedOutElsewhere,
};
use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, MatrixDeviceId, MatrixFailure, MatrixFailureKind, MatrixHumanDeviceSignOut,
        MatrixOperation, MatrixResult, MatrixUserId, OidcFailure, OidcFailureKind, OidcResult,
        OtherSessionsRevoked, PortFuture, SecretDigest, SecretFactory, SecretGenerationFailure,
        SecretValue, SignInAccount, SignInSessionTermination, StoredWebSession, WebSessionStore,
    },
};

const NOW: i64 = 1_790_000_000_000;
const USER: &str = "@user-0123456789abcdef0123456789abcdef:matrix.agent-room.localhost";

fn account() -> SignInAccount {
    SignInAccount {
        issuer: "https://identity.agent-room.localhost/realms/agent-room".to_owned(),
        subject: "subject".to_owned(),
    }
}

/// 记下撤了谁的、留着哪份；`unavailable` 时像库连不上。
#[derive(Default)]
struct Sessions {
    calls: Mutex<Vec<(PrincipalId, Option<SecretDigest>, UtcMillis)>>,
    unavailable: Mutex<bool>,
}

impl WebSessionStore for Sessions {
    fn find_active<'a>(
        &'a self,
        _secret_digest: &'a SecretDigest,
        _now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<StoredWebSession>>> {
        unreachable!("退出别的地方不查登录")
    }

    fn revoke<'a>(
        &'a self,
        _secret_digest: &'a SecretDigest,
        _now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<SignInAccount>>> {
        unreachable!("退出别的地方不撤正在用的这份")
    }

    fn revoke_others<'a>(
        &'a self,
        principal_id: PrincipalId,
        keep: Option<&'a SecretDigest>,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<OtherSessionsRevoked>>> {
        self.calls
            .lock()
            .unwrap()
            .push((principal_id, keep.copied(), now));
        let result = if *self.unavailable.lock().unwrap() {
            Err(RepositoryError::new(
                "test.web_session.revoke_others",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            Ok(Some(OtherSessionsRevoked {
                revoked: 3,
                account: account(),
            }))
        };
        Box::pin(async move { result })
    }

    fn extend(
        &self,
        _session_id: WebSessionId,
        _expires_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        unreachable!("退出别的地方不续登录")
    }
}

#[derive(Default)]
struct ChatDevices {
    calls: Mutex<Vec<(String, Option<String>)>>,
    unavailable: Mutex<bool>,
}

impl MatrixHumanDeviceSignOut for ChatDevices {
    fn sign_out_other_devices<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
        keep: Option<&'a MatrixDeviceId>,
    ) -> PortFuture<'a, MatrixResult<usize>> {
        self.calls.lock().unwrap().push((
            user_id.as_str().to_owned(),
            keep.map(|device| device.as_str().to_owned()),
        ));
        let result = if *self.unavailable.lock().unwrap() {
            Err(MatrixFailure::new(
                MatrixOperation::SignOutHumanDevices,
                MatrixFailureKind::DependencyUnavailable,
            ))
        } else {
            Ok(2)
        };
        Box::pin(async move { result })
    }
}

#[derive(Default)]
struct SignInSessions {
    ended: Mutex<Vec<SignInAccount>>,
    unavailable: Mutex<bool>,
}

impl SignInSessionTermination for SignInSessions {
    fn end_sessions<'a>(&'a self, account: &'a SignInAccount) -> PortFuture<'a, OidcResult<()>> {
        self.ended.lock().unwrap().push(account.clone());
        let result = if *self.unavailable.lock().unwrap() {
            Err(OidcFailure::new(OidcFailureKind::DependencyUnavailable))
        } else {
            Ok(())
        };
        Box::pin(async move { result })
    }
}

struct Secrets;

impl SecretFactory for Secrets {
    fn generate(&self) -> Result<SecretValue, SecretGenerationFailure> {
        unreachable!("退出别的地方不发新秘密")
    }

    fn digest(&self, value: &str) -> SecretDigest {
        let mut digest = [0_u8; 32];
        for (slot, byte) in digest.iter_mut().zip(value.bytes()) {
            *slot = byte;
        }
        SecretDigest::from_array(digest)
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(NOW).unwrap()
    }
}

struct Harness {
    sessions: Arc<Sessions>,
    chat_devices: Arc<ChatDevices>,
    sign_in_sessions: Arc<SignInSessions>,
    service: SignOutElsewhereService,
}

fn harness() -> Harness {
    let sessions = Arc::new(Sessions::default());
    let chat_devices = Arc::new(ChatDevices::default());
    let sign_in_sessions = Arc::new(SignInSessions::default());
    let service = SignOutElsewhereService::new(SignOutElsewhereDependencies {
        sessions: sessions.clone(),
        chat_devices: chat_devices.clone(),
        sign_in_sessions: sign_in_sessions.clone(),
        secrets: Arc::new(Secrets),
        clock: Arc::new(FixedClock),
    });
    Harness {
        sessions,
        chat_devices,
        sign_in_sessions,
        service,
    }
}

fn actor() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
        matrix_user_id: USER.to_owned(),
        display_name: "林雨".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: UtcMillis::new(NOW).unwrap(),
        expires_at: UtcMillis::new(NOW + 86_400_000).unwrap(),
        recently_authenticated: false,
    }
}

fn secret() -> SecretValue {
    SecretValue::new("this-session").unwrap()
}

#[tokio::test]
async fn 撤掉别的账户登录_结束身份服务上的登录_注销别的聊天设备_这一份和这一台留着() {
    let harness = harness();
    let actor = actor();
    let keep = MatrixDeviceId::new("THISBROWSER").unwrap();

    let signed_out = harness
        .service
        .sign_out(&actor, &secret(), Some(&keep))
        .await
        .unwrap();

    assert_eq!(
        signed_out,
        SignedOutElsewhere {
            account_sessions: 3,
            chat_devices: 2,
            sign_in_sessions_ended: true,
        }
    );
    assert_eq!(
        *harness.sessions.calls.lock().unwrap(),
        [(
            actor.principal_id,
            Some(Secrets.digest("this-session")),
            UtcMillis::new(NOW).unwrap()
        )]
    );
    assert_eq!(*harness.sign_in_sessions.ended.lock().unwrap(), [account()]);
    assert_eq!(
        *harness.chat_devices.calls.lock().unwrap(),
        [(USER.to_owned(), Some("THISBROWSER".to_owned()))]
    );
}

#[tokio::test]
async fn 这一台还没有聊天登录时别的聊天设备全部注销() {
    let harness = harness();

    harness
        .service
        .sign_out(&actor(), &secret(), None)
        .await
        .unwrap();

    assert_eq!(
        *harness.chat_devices.calls.lock().unwrap(),
        [(USER.to_owned(), None)]
    );
}

#[tokio::test]
async fn 库连不上时什么都不退() {
    let harness = harness();
    *harness.sessions.unavailable.lock().unwrap() = true;

    let failure = harness
        .service
        .sign_out(&actor(), &secret(), None)
        .await
        .unwrap_err();

    assert_eq!(failure, SignOutElsewhereFailure::AccountSessionsUnavailable);
    assert!(harness.sign_in_sessions.ended.lock().unwrap().is_empty());
    assert!(harness.chat_devices.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 聊天服务器连不上时账户登录已经撤了_报聊天不可用() {
    let harness = harness();
    *harness.chat_devices.unavailable.lock().unwrap() = true;

    let failure = harness
        .service
        .sign_out(&actor(), &secret(), None)
        .await
        .unwrap_err();

    assert_eq!(failure, SignOutElsewhereFailure::ChatUnavailable);
    assert_eq!(harness.sessions.calls.lock().unwrap().len(), 1);
    assert_eq!(harness.sign_in_sessions.ended.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn 身份服务上的登录没结束成不算失败() {
    let harness = harness();
    *harness.sign_in_sessions.unavailable.lock().unwrap() = true;

    let signed_out = harness
        .service
        .sign_out(&actor(), &secret(), None)
        .await
        .unwrap();

    assert!(!signed_out.sign_in_sessions_ended);
    assert_eq!(signed_out.chat_devices, 2);
}
