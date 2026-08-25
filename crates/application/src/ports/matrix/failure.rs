use agent_room_domain::{DomainError, DomainResult, time::DurationMillis};

const MAX_RETRY_ATTEMPTS: u16 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixOperation {
    InitializeStore,
    ProvisionAgentUser,
    IssueAgentDeviceSession,
    Login,
    RestoreSession,
    Sync,
    CreateRoom,
    ResolveRoomAlias,
    ReadAccountData,
    SetAccountData,
    InspectMembership,
    Invite,
    Join,
    Leave,
    Kick,
    Ban,
    UpdatePowerLevels,
    ArchiveRoom,
    SendEvent,
    SendStateEvent,
    SendReceipt,
    Backfill,
    InspectRoomAuthority,
}

impl MatrixOperation {
    pub const fn is_safe_to_retry(self) -> bool {
        matches!(
            self,
            Self::InitializeStore
                | Self::ProvisionAgentUser
                | Self::Login
                | Self::RestoreSession
                | Self::Sync
                | Self::ResolveRoomAlias
                | Self::ReadAccountData
                | Self::SetAccountData
                | Self::InspectMembership
                | Self::Invite
                | Self::Join
                | Self::Leave
                | Self::Kick
                | Self::Ban
                | Self::UpdatePowerLevels
                | Self::ArchiveRoom
                | Self::SendStateEvent
                | Self::SendReceipt
                | Self::Backfill
                | Self::InspectRoomAuthority
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixFailureKind {
    InvalidConfiguration,
    Unauthenticated,
    AuthenticationRejected,
    Forbidden,
    NotFound,
    Conflict,
    RateLimited,
    Timeout,
    DependencyUnavailable,
    InvalidResponse,
    UnknownCommit,
    StaleSyncToken,
    UnsupportedVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatrixFailure {
    operation: MatrixOperation,
    kind: MatrixFailureKind,
    retry_after: Option<DurationMillis>,
}

impl MatrixFailure {
    pub const fn new(operation: MatrixOperation, kind: MatrixFailureKind) -> Self {
        Self {
            operation,
            kind,
            retry_after: None,
        }
    }

    pub const fn rate_limited(
        operation: MatrixOperation,
        retry_after: Option<DurationMillis>,
    ) -> Self {
        Self {
            operation,
            kind: MatrixFailureKind::RateLimited,
            retry_after,
        }
    }

    pub const fn operation(self) -> MatrixOperation {
        self.operation
    }

    pub const fn kind(self) -> MatrixFailureKind {
        self.kind
    }

    pub const fn retry_after(self) -> Option<DurationMillis> {
        self.retry_after
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixRecoveryAction {
    RetryAfter(DurationMillis),
    ReconcileSubmission,
    ResetSyncCursor,
    Reauthenticate,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatrixRetryPolicy {
    initial_delay: DurationMillis,
    maximum_delay: DurationMillis,
    maximum_attempts: u16,
}

impl MatrixRetryPolicy {
    /// 创建有界指数退避策略。
    ///
    /// # Errors
    ///
    /// 初始延迟大于最大延迟，或尝试次数不在安全范围内时返回校验错误。
    pub fn new(
        initial_delay: DurationMillis,
        maximum_delay: DurationMillis,
        maximum_attempts: u16,
    ) -> DomainResult<Self> {
        if initial_delay > maximum_delay {
            return Err(DomainError::Validation {
                field: "matrix_retry_delay",
                reason: "初始延迟不能大于最大延迟",
            });
        }
        if maximum_attempts == 0 || maximum_attempts > MAX_RETRY_ATTEMPTS {
            return Err(DomainError::Validation {
                field: "matrix_retry_attempts",
                reason: "必须处于 1 到 32 之间",
            });
        }
        Ok(Self {
            initial_delay,
            maximum_delay,
            maximum_attempts,
        })
    }

    pub fn recovery(self, failure: MatrixFailure, completed_attempts: u16) -> MatrixRecoveryAction {
        match failure.kind() {
            MatrixFailureKind::UnknownCommit => MatrixRecoveryAction::ReconcileSubmission,
            MatrixFailureKind::StaleSyncToken => MatrixRecoveryAction::ResetSyncCursor,
            MatrixFailureKind::Unauthenticated => MatrixRecoveryAction::Reauthenticate,
            MatrixFailureKind::RateLimited if completed_attempts < self.maximum_attempts => {
                let delay = failure
                    .retry_after()
                    .unwrap_or_else(|| self.exponential_delay(completed_attempts));
                MatrixRecoveryAction::RetryAfter(self.cap(delay))
            }
            MatrixFailureKind::Timeout | MatrixFailureKind::DependencyUnavailable
                if failure.operation().is_safe_to_retry()
                    && completed_attempts < self.maximum_attempts =>
            {
                let delay = failure
                    .retry_after()
                    .unwrap_or_else(|| self.exponential_delay(completed_attempts));
                MatrixRecoveryAction::RetryAfter(self.cap(delay))
            }
            _ => MatrixRecoveryAction::Stop,
        }
    }

    fn exponential_delay(self, completed_attempts: u16) -> DurationMillis {
        let shift = u32::from(completed_attempts.saturating_sub(1)).min(63);
        let multiplier = 1_u64.checked_shl(shift).unwrap_or(u64::MAX);
        let millis = self
            .initial_delay
            .value()
            .saturating_mul(multiplier)
            .min(self.maximum_delay.value());
        DurationMillis::new(millis).expect("退避策略保证延迟非零")
    }

    fn cap(self, delay: DurationMillis) -> DurationMillis {
        DurationMillis::new(delay.value().min(self.maximum_delay.value()))
            .expect("退避策略保证延迟非零")
    }
}

#[cfg(test)]
mod tests {
    use agent_room_domain::time::DurationMillis;

    use super::{
        MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixRecoveryAction, MatrixRetryPolicy,
    };

    fn policy() -> MatrixRetryPolicy {
        MatrixRetryPolicy::new(
            DurationMillis::new(250).expect("延迟有效"),
            DurationMillis::new(4_000).expect("延迟有效"),
            4,
        )
        .expect("策略有效")
    }

    #[test]
    fn 退避有上限且服务端延迟同样受本地上限约束() {
        let transient = MatrixFailure::new(
            MatrixOperation::Sync,
            MatrixFailureKind::DependencyUnavailable,
        );
        assert_eq!(
            policy().recovery(transient, 1),
            MatrixRecoveryAction::RetryAfter(DurationMillis::new(250).expect("延迟有效"))
        );
        assert_eq!(policy().recovery(transient, 4), MatrixRecoveryAction::Stop);

        let limited = MatrixFailure::rate_limited(
            MatrixOperation::Sync,
            Some(DurationMillis::new(60_000).expect("延迟有效")),
        );
        assert_eq!(
            policy().recovery(limited, 1),
            MatrixRecoveryAction::RetryAfter(DurationMillis::new(4_000).expect("延迟有效"))
        );

        let rejected_send = MatrixFailure::rate_limited(MatrixOperation::SendEvent, None);
        assert!(matches!(
            policy().recovery(rejected_send, 1),
            MatrixRecoveryAction::RetryAfter(_)
        ));
    }

    #[test]
    fn 非幂等未知提交进入对账而不是盲目重试() {
        let create_unknown = MatrixFailure::new(
            MatrixOperation::CreateRoom,
            MatrixFailureKind::UnknownCommit,
        );
        let send_unknown =
            MatrixFailure::new(MatrixOperation::SendEvent, MatrixFailureKind::UnknownCommit);

        assert_eq!(
            policy().recovery(create_unknown, 1),
            MatrixRecoveryAction::ReconcileSubmission
        );
        assert_eq!(
            policy().recovery(send_unknown, 1),
            MatrixRecoveryAction::ReconcileSubmission
        );
    }

    #[test]
    fn 失效游标和失效认证使用不同恢复路径() {
        assert_eq!(
            policy().recovery(
                MatrixFailure::new(MatrixOperation::Sync, MatrixFailureKind::StaleSyncToken),
                1,
            ),
            MatrixRecoveryAction::ResetSyncCursor
        );
        assert_eq!(
            policy().recovery(
                MatrixFailure::new(MatrixOperation::Sync, MatrixFailureKind::Unauthenticated),
                1,
            ),
            MatrixRecoveryAction::Reauthenticate
        );
    }
}
