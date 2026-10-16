//! 桌面端的聊天登录由控制面替人发（`specs/desktop-sign-in/design.md`）：以应用服务的身份替这个人
//! 登录 Matrix，不再开第二次浏览器、停在 Synapse 的确认页。只认桌面登录，由接口那边把关。

use std::sync::Arc;

use agent_room_domain::time::UtcMillis;

use crate::{
    authentication::AuthenticatedPrincipal,
    hourly_limit::HourlyLimit,
    ports::{Clock, MatrixHumanSessionIssuer, MatrixSession, MatrixUserId},
};

/// 一小时里最多发几次：每次都是 Synapse 上的一台新设备，客户端出了循环时挡一挡。
pub const SESSIONS_PER_HOUR: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopMatrixSessionFailure {
    /// 发得太勤：`retry_at` 之后再来。
    RateLimited { retry_at: UtcMillis },
    /// Synapse 暂时不可用，或者回答不对。
    Unavailable,
}

pub struct DesktopMatrixSessionService {
    matrix: Arc<dyn MatrixHumanSessionIssuer>,
    clock: Arc<dyn Clock>,
    issued: HourlyLimit,
}

impl DesktopMatrixSessionService {
    pub fn new(matrix: Arc<dyn MatrixHumanSessionIssuer>, clock: Arc<dyn Clock>) -> Self {
        Self {
            matrix,
            clock,
            issued: HourlyLimit::new(SESSIONS_PER_HOUR),
        }
    }

    /// 替本人登录 Matrix，拿一台带刷新令牌的新设备。还没有 Matrix 账户时由适配器先注册。
    ///
    /// # Errors
    ///
    /// 一小时里已经发过 [`SESSIONS_PER_HOUR`] 次时返回 `RateLimited`；Synapse 不可用时返回
    /// `Unavailable`。
    pub async fn issue(
        &self,
        actor: &AuthenticatedPrincipal,
    ) -> Result<MatrixSession, DesktopMatrixSessionFailure> {
        let user = MatrixUserId::new(actor.matrix_user_id.clone())
            .map_err(|_| DesktopMatrixSessionFailure::Unavailable)?;
        self.issued
            .take(actor.principal_id, self.clock.now())
            .map_err(|reached| DesktopMatrixSessionFailure::RateLimited {
                retry_at: reached.retry_at,
            })?;
        self.matrix
            .issue_human_session(&user, &actor.display_name)
            .await
            .map_err(|_| DesktopMatrixSessionFailure::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use agent_room_domain::{ids::PrincipalId, time::UtcMillis};
    use uuid::Uuid;

    use super::{DesktopMatrixSessionFailure, DesktopMatrixSessionService, SESSIONS_PER_HOUR};
    use crate::{
        authentication::AuthenticatedPrincipal,
        ports::{
            Clock, MatrixDeviceId, MatrixFailure, MatrixFailureKind, MatrixHumanSessionIssuer,
            MatrixOperation, MatrixResult, MatrixSession, MatrixSessionMetadata, MatrixUserId,
            PortFuture, SecretValue,
        },
    };

    const NOW: i64 = 1_790_000_000_000;
    const USER: &str = "@user-0123456789abcdef0123456789abcdef:matrix.agent-room.localhost";

    #[derive(Default)]
    struct Synapse {
        calls: Mutex<Vec<(String, String)>>,
        unavailable: Mutex<bool>,
    }

    impl MatrixHumanSessionIssuer for Synapse {
        fn issue_human_session<'a>(
            &'a self,
            user_id: &'a MatrixUserId,
            display_name: &'a str,
        ) -> PortFuture<'a, MatrixResult<MatrixSession>> {
            self.calls
                .lock()
                .unwrap()
                .push((user_id.as_str().to_owned(), display_name.to_owned()));
            let result = if *self.unavailable.lock().unwrap() {
                Err(MatrixFailure::new(
                    MatrixOperation::IssueHumanSession,
                    MatrixFailureKind::DependencyUnavailable,
                ))
            } else {
                Ok(MatrixSession::new(
                    MatrixSessionMetadata::new(
                        user_id.clone(),
                        MatrixDeviceId::new("DESKTOPDEV").unwrap(),
                    ),
                    SecretValue::new("syt_access".to_owned()).unwrap(),
                    Some(SecretValue::new("syr_refresh".to_owned()).unwrap()),
                ))
            };
            Box::pin(async move { result })
        }
    }

    struct FixedClock(Mutex<i64>);

    impl Clock for FixedClock {
        fn now(&self) -> UtcMillis {
            UtcMillis::new(*self.0.lock().unwrap()).unwrap()
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
            recently_authenticated: true,
        }
    }

    fn service(synapse: &Arc<Synapse>, clock: &Arc<FixedClock>) -> DesktopMatrixSessionService {
        DesktopMatrixSessionService::new(synapse.clone(), clock.clone())
    }

    #[tokio::test]
    async fn 替本人登录_带上账户的显示名() {
        let synapse = Arc::new(Synapse::default());
        let clock = Arc::new(FixedClock(Mutex::new(NOW)));
        let actor = actor();

        let session = service(&synapse, &clock)
            .issue(&actor)
            .await
            .expect("发出聊天登录");

        assert_eq!(session.metadata().user_id().as_str(), USER);
        assert_eq!(session.access_token().expose(), "syt_access");
        assert_eq!(
            session.refresh_token().map(SecretValue::expose),
            Some("syr_refresh")
        );
        assert_eq!(
            synapse.calls.lock().unwrap().as_slice(),
            [(USER.to_owned(), "林雨".to_owned())]
        );
    }

    #[tokio::test]
    async fn 一小时里发够次数以后要等_过了一小时又能发() {
        let synapse = Arc::new(Synapse::default());
        let clock = Arc::new(FixedClock(Mutex::new(NOW)));
        let service = service(&synapse, &clock);
        let actor = actor();
        for _ in 0..SESSIONS_PER_HOUR {
            service.issue(&actor).await.expect("限额以内");
        }

        assert_eq!(
            service.issue(&actor).await.map(|_| ()),
            Err(DesktopMatrixSessionFailure::RateLimited {
                retry_at: UtcMillis::new(NOW + 3_600_000).unwrap()
            })
        );
        assert_eq!(synapse.calls.lock().unwrap().len(), SESSIONS_PER_HOUR);

        *clock.0.lock().unwrap() = NOW + 3_600_000;
        service.issue(&actor).await.expect("过了一小时");
    }

    #[tokio::test]
    async fn synapse_不可用时照实说() {
        let synapse = Arc::new(Synapse::default());
        *synapse.unavailable.lock().unwrap() = true;
        let clock = Arc::new(FixedClock(Mutex::new(NOW)));

        assert_eq!(
            service(&synapse, &clock).issue(&actor()).await.map(|_| ()),
            Err(DesktopMatrixSessionFailure::Unavailable)
        );
    }
}
