//! 服务器开没开 Matrix 在线状态（`specs/agent-liveness/design.md` 第 3 步）。报一次此刻该报的，
//! 再读回自己的就知道：开着才写名片、报在线状态，没开就照旧写租约。本机 Bridge 和网关都用。

use agent_room_application::ports::{MatrixFailure, MatrixFailureKind, MatrixResult, PortFuture};
use agent_room_domain::agent_lifecycle::MatrixPresenceState;

/// 读回来还是离线，有这么多次才当没开：报完马上就读，万一服务器还没记上。
const OFFLINE_READS_BEFORE_DISABLED: u8 = 2;

/// 报、读自己的 Matrix 在线状态。
pub trait OwnPresence: Send + Sync {
    /// `PUT /presence/{自己}/status`。
    fn report(&self, presence: MatrixPresenceState) -> PortFuture<'_, MatrixResult<()>>;

    /// `GET /presence/{自己}/status`。
    fn read(&self) -> PortFuture<'_, MatrixResult<MatrixPresenceState>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceSupport {
    /// 还不知道。`offline_reads` 是报完读回来还是离线的次数。
    Unknown {
        offline_reads: u8,
    },
    Enabled,
    Disabled,
}

impl Default for PresenceSupport {
    fn default() -> Self {
        Self::Unknown { offline_reads: 0 }
    }
}

impl PresenceSupport {
    /// 还不知道就探一次：报此刻该报的，再读回自己的。读回来不是离线就是开着；报了、读回来还是
    /// 离线，两次才算没开；服务器不接这两个接口（404、403、版本不支持），一次就算没开。
    ///
    /// 报不出去（限速、超时、断网）也照样读：之前报的或者同步带的已经记上了，读回来不是离线
    /// 就看得出开着。Synapse 每个用户 10 秒只认一次报，刚进房间就接着开始等消息的，第二次探
    /// 总是被限速。没报成时读回离线说明不了什么，不算数。还是不知道就把错交回去记日志，下次
    /// 再探。已经知道了就什么都不做。
    pub async fn probe(
        &mut self,
        own: &dyn OwnPresence,
        wanted: MatrixPresenceState,
    ) -> Option<MatrixFailure> {
        let Self::Unknown { offline_reads } = *self else {
            return None;
        };
        let unreported = match own.report(wanted).await {
            Ok(()) => None,
            Err(failure) if unsupported(failure.kind()) => {
                *self = Self::Disabled;
                return None;
            }
            Err(failure) => Some(failure),
        };
        match own.read().await {
            Ok(MatrixPresenceState::Offline) => {
                if unreported.is_some() {
                    return unreported;
                }
                let offline_reads = offline_reads.saturating_add(1);
                *self = if offline_reads >= OFFLINE_READS_BEFORE_DISABLED {
                    Self::Disabled
                } else {
                    Self::Unknown { offline_reads }
                };
                None
            }
            Ok(_) => {
                *self = Self::Enabled;
                None
            }
            Err(failure) if unsupported(failure.kind()) => {
                *self = Self::Disabled;
                None
            }
            Err(failure) => Some(unreported.unwrap_or(failure)),
        }
    }
}

const fn unsupported(kind: MatrixFailureKind) -> bool {
    matches!(
        kind,
        MatrixFailureKind::NotFound
            | MatrixFailureKind::Forbidden
            | MatrixFailureKind::UnsupportedVersion
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use agent_room_application::ports::MatrixOperation;

    use super::*;
    use MatrixPresenceState::{Offline, Online, Unavailable};

    /// 按顺序给出每次报的结果和读回来的状态。
    struct 脚本(Mutex<Vec<Result<MatrixPresenceState, MatrixFailureKind>>>);

    impl 脚本 {
        fn new(
            steps: impl IntoIterator<Item = Result<MatrixPresenceState, MatrixFailureKind>>,
        ) -> Self {
            Self(Mutex::new(steps.into_iter().collect()))
        }

        fn next(&self) -> Result<MatrixPresenceState, MatrixFailureKind> {
            self.0.lock().expect("锁可用").remove(0)
        }
    }

    impl OwnPresence for 脚本 {
        fn report(&self, _presence: MatrixPresenceState) -> PortFuture<'_, MatrixResult<()>> {
            let result = match self.next() {
                Err(kind) => Err(MatrixFailure::new(MatrixOperation::ReportPresence, kind)),
                Ok(_) => Ok(()),
            };
            Box::pin(async move { result })
        }

        fn read(&self) -> PortFuture<'_, MatrixResult<MatrixPresenceState>> {
            let result = self
                .next()
                .map_err(|kind| MatrixFailure::new(MatrixOperation::ReadPresence, kind));
            Box::pin(async move { result })
        }
    }

    async fn 探(support: &mut PresenceSupport, own: &脚本) -> Option<MatrixFailureKind> {
        support
            .probe(own, Unavailable)
            .await
            .map(MatrixFailure::kind)
    }

    #[tokio::test]
    async fn 读回来不是离线就是开着_之后不再探() {
        let own = 脚本::new([Ok(Unavailable), Ok(Unavailable)]);
        let mut support = PresenceSupport::default();
        assert_eq!(探(&mut support, &own).await, None);
        assert_eq!(support, PresenceSupport::Enabled);
        assert_eq!(探(&mut support, &own).await, None);
        assert!(
            own.0.lock().expect("锁可用").is_empty(),
            "开着以后不再报和读"
        );
    }

    #[tokio::test]
    async fn 读回来还是离线_两次才算没开() {
        let own = 脚本::new([Ok(Online), Ok(Offline), Ok(Online), Ok(Offline)]);
        let mut support = PresenceSupport::default();
        探(&mut support, &own).await;
        assert_eq!(support, PresenceSupport::Unknown { offline_reads: 1 });
        探(&mut support, &own).await;
        assert_eq!(support, PresenceSupport::Disabled);
    }

    #[tokio::test]
    async fn 接口不认一次就算没开_限速只是这次不知道() {
        for kind in [
            MatrixFailureKind::NotFound,
            MatrixFailureKind::Forbidden,
            MatrixFailureKind::UnsupportedVersion,
        ] {
            let own = 脚本::new([Err(kind)]);
            let mut support = PresenceSupport::default();
            assert_eq!(探(&mut support, &own).await, None);
            assert_eq!(support, PresenceSupport::Disabled);
        }
        let own = 脚本::new([
            Err(MatrixFailureKind::RateLimited),
            Err(MatrixFailureKind::Timeout),
            Ok(Online),
            Err(MatrixFailureKind::Timeout),
            Ok(Online),
            Ok(Online),
        ]);
        let mut support = PresenceSupport::default();
        assert_eq!(
            探(&mut support, &own).await,
            Some(MatrixFailureKind::RateLimited),
            "报不出去、读也超时：交回报的那个错"
        );
        assert_eq!(
            探(&mut support, &own).await,
            Some(MatrixFailureKind::Timeout),
            "报成了、读回来超时"
        );
        assert_eq!(support, PresenceSupport::default());
        探(&mut support, &own).await;
        assert_eq!(support, PresenceSupport::Enabled);
    }

    #[tokio::test]
    async fn 报不出去也读回来看_不是离线就是开着_离线不算数() {
        // 被限速、读回离线：说明不了什么，不算一次。
        let own = 脚本::new([
            Err(MatrixFailureKind::RateLimited),
            Ok(Offline),
            Ok(Online),
            Ok(Offline),
            Err(MatrixFailureKind::RateLimited),
            Ok(Offline),
        ]);
        let mut support = PresenceSupport::default();
        assert_eq!(
            探(&mut support, &own).await,
            Some(MatrixFailureKind::RateLimited)
        );
        assert_eq!(support, PresenceSupport::default());
        探(&mut support, &own).await;
        assert_eq!(support, PresenceSupport::Unknown { offline_reads: 1 });
        探(&mut support, &own).await;
        assert_eq!(
            support,
            PresenceSupport::Unknown { offline_reads: 1 },
            "报了读回离线才算数"
        );

        // 被限速、读回来是之前报的或者同步带的：开着。
        let own = 脚本::new([Err(MatrixFailureKind::RateLimited), Ok(Unavailable)]);
        let mut support = PresenceSupport::Unknown { offline_reads: 1 };
        assert_eq!(探(&mut support, &own).await, None);
        assert_eq!(support, PresenceSupport::Enabled);
    }
}
