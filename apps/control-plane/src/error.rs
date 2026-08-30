use std::collections::BTreeMap;

use agent_room_application::{
    account_lifecycle::{AccountLifecycleFailure, AccountLifecycleFailureKind},
    agent_cards::{AgentCardManagementFailure, AgentCardManagementFailureKind},
    agent_instance_management::{
        AgentInstanceManagementFailure, AgentInstanceManagementFailureKind,
    },
    agent_instance_verification::{
        AgentInstanceVerificationFailure, AgentInstanceVerificationFailureKind,
    },
    agent_lobbies::{AgentLobbyEntryFailure, AgentLobbyEntryFailureKind},
    agents::{AgentManagementFailure, AgentManagementFailureKind},
    authentication::{AuthenticationFailure, AuthenticationFailureKind},
    automation::{AutomationFailure, AutomationFailureKind},
    content::{
        BeginContentUploadFailure, BindContentEventFailure, CompleteContentUploadFailure,
        IssueContentReadTicketFailure, OpenContentFailure, RedactContentFailure,
    },
    devices::{DeviceAuthorizationFailure, DeviceAuthorizationFailureKind},
    direct_sessions::{DirectSessionFailure, DirectSessionFailureKind},
    handoffs::{
        HandoffAccessFailure, HandoffAccessFailureKind, TargetedHandoffFailure,
        TargetedHandoffFailureKind,
    },
    moderation::{ModerationFailure, ModerationFailureKind},
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        ContentAuthorizationFailure, ContentAuthorizationFailureKind, ContentScanFailureKind,
        ContentTicketFailure, ContentTicketFailureKind, ObjectStoreFailureKind,
    },
    private_rooms::{PrivateRoomFailure, PrivateRoomFailureKind},
};
use agent_room_domain::content::ContentLifecycleState;
use agent_room_protocol_conformance::generated::{ErrorCategory, ErrorEnvelope};
use axum::{
    Json,
    extract::Extension,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::correlation::CorrelationId;

pub(crate) struct ApiError {
    status: StatusCode,
    envelope: ErrorEnvelope,
}

impl ApiError {
    pub(crate) fn code(&self) -> &str {
        &self.envelope.code
    }

    pub(crate) fn correlation_id(&self) -> &str {
        &self.envelope.correlation_id
    }

    pub(crate) fn new(
        status: StatusCode,
        code: &str,
        category: ErrorCategory,
        message: &str,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            status,
            envelope: ErrorEnvelope {
                category,
                code: code.to_owned(),
                correlation_id: correlation_id.as_uuid().to_string(),
                details: BTreeMap::new(),
                message: message.to_owned(),
                retryable: false,
                retry_after_seconds: None,
                extensions: BTreeMap::new(),
            },
        }
    }

    pub(crate) fn invalid_request(code: &str, correlation_id: CorrelationId) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            code,
            ErrorCategory::Validation,
            "请求无法通过安全校验。",
            correlation_id,
        )
    }

    pub(crate) fn account(failure: AccountLifecycleFailure, correlation_id: CorrelationId) -> Self {
        let (status, code, category, message) = match failure.kind() {
            AccountLifecycleFailureKind::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "account.invalid_request",
                ErrorCategory::Validation,
                "账户生命周期请求无效。",
            ),
            AccountLifecycleFailureKind::Forbidden => (
                StatusCode::FORBIDDEN,
                "account.forbidden",
                ErrorCategory::Authorization,
                "当前会话无权执行该账户操作。",
            ),
            AccountLifecycleFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "account.not_found",
                ErrorCategory::Validation,
                "账户导出或删除回执不存在。",
            ),
            AccountLifecycleFailureKind::Conflict => (
                StatusCode::CONFLICT,
                "account.conflict",
                ErrorCategory::Conflict,
                "账户删除已开始；请使用首次返回的删除回执查询进度。",
            ),
            AccountLifecycleFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "account.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "账户生命周期依赖暂时不可用。",
            ),
            AccountLifecycleFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "account.internal",
                ErrorCategory::Transient,
                "账户生命周期服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            failure = ?failure.kind(),
            "账户生命周期请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn automation(failure: AutomationFailure, correlation_id: CorrelationId) -> Self {
        let (status, code, category, message) = match failure.kind() {
            AutomationFailureKind::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "automation.invalid_request",
                ErrorCategory::Validation,
                "自动发言授权请求无效。",
            ),
            AutomationFailureKind::Forbidden => (
                StatusCode::FORBIDDEN,
                "automation.forbidden",
                ErrorCategory::Authorization,
                "当前主体无权执行该自动发言授权操作。",
            ),
            AutomationFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "automation.not_found",
                ErrorCategory::Validation,
                "自动发言授权不存在。",
            ),
            AutomationFailureKind::Conflict => (
                StatusCode::CONFLICT,
                "automation.conflict",
                ErrorCategory::Conflict,
                "自动发言授权状态已经变化，请刷新后重试。",
            ),
            AutomationFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "automation.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "自动发言授权依赖暂时不可用，发送已拒绝。",
            ),
            AutomationFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "automation.internal",
                ErrorCategory::Transient,
                "自动发言授权服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            failure = ?failure.kind(),
            "自动发言授权请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn moderation(failure: ModerationFailure, correlation_id: CorrelationId) -> Self {
        let mapping = match failure.kind() {
            ModerationFailureKind::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "moderation.invalid_request",
                ErrorCategory::Validation,
                "治理请求无效。",
            ),
            ModerationFailureKind::Forbidden => (
                StatusCode::FORBIDDEN,
                "moderation.forbidden",
                ErrorCategory::Authorization,
                "当前主体无权执行该治理操作。",
            ),
            ModerationFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "moderation.not_found",
                ErrorCategory::Validation,
                "治理资源不存在。",
            ),
            ModerationFailureKind::Conflict => (
                StatusCode::CONFLICT,
                "moderation.conflict",
                ErrorCategory::Conflict,
                "治理状态已经变化，请刷新后重试。",
            ),
            ModerationFailureKind::RateLimited => {
                let error = Self::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "moderation.rate_limited",
                    ErrorCategory::Transient,
                    "举报过于频繁，请稍后重试。",
                    correlation_id,
                )
                .retry_after_seconds(seconds_until(
                    failure.retry_at().expect("限速失败必须携带重试时间"),
                ));
                log_moderation_failure(failure, correlation_id);
                return error;
            }
            ModerationFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "moderation.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "治理依赖暂时不可用，动作未被伪装为成功。",
            ),
            ModerationFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "moderation.internal",
                ErrorCategory::Transient,
                "治理服务发生内部错误。",
            ),
        };
        log_moderation_failure(failure, correlation_id);
        from_mapping(mapping, correlation_id)
    }

    fn retry_after_seconds(mut self, seconds: u64) -> Self {
        self.envelope.retryable = true;
        self.envelope.retry_after_seconds = Some(seconds);
        self
    }

    pub(crate) fn authentication(
        failure: AuthenticationFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let (status, code, category, message) = match failure.kind() {
            AuthenticationFailureKind::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "authentication.invalid_request",
                ErrorCategory::Validation,
                "认证请求无效。",
            ),
            AuthenticationFailureKind::InvalidLoginState => (
                StatusCode::BAD_REQUEST,
                "authentication.invalid_login_state",
                ErrorCategory::Validation,
                "登录状态已失效，请重新开始登录。",
            ),
            AuthenticationFailureKind::ProviderRejected => (
                StatusCode::UNAUTHORIZED,
                "authentication.provider_rejected",
                ErrorCategory::Authentication,
                "身份提供方拒绝了登录。",
            ),
            AuthenticationFailureKind::InvalidIdentityToken => (
                StatusCode::UNAUTHORIZED,
                "authentication.invalid_identity_token",
                ErrorCategory::Authentication,
                "身份声明无法验证。",
            ),
            AuthenticationFailureKind::InvalidSession => (
                StatusCode::UNAUTHORIZED,
                "authentication.invalid_session",
                ErrorCategory::Authentication,
                "会话无效或已过期。",
            ),
            AuthenticationFailureKind::PrincipalSuspended => (
                StatusCode::FORBIDDEN,
                "authentication.principal_suspended",
                ErrorCategory::Authorization,
                "该主体已暂停。",
            ),
            AuthenticationFailureKind::ReauthenticationRequired => (
                StatusCode::UNAUTHORIZED,
                "authentication.reauthentication_required",
                ErrorCategory::Authentication,
                "该操作需要近期重新认证。",
            ),
            AuthenticationFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "authentication.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "认证依赖暂时不可用。",
            ),
            AuthenticationFailureKind::Conflict => (
                StatusCode::CONFLICT,
                "authentication.conflict",
                ErrorCategory::Conflict,
                "认证状态发生冲突，请重试。",
            ),
            AuthenticationFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "authentication.internal",
                ErrorCategory::Transient,
                "认证服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            failure = ?failure.kind(),
            "认证请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn device(
        failure: DeviceAuthorizationFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let (status, code, category, message) = match failure.kind() {
            DeviceAuthorizationFailureKind::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "device.invalid_request",
                ErrorCategory::Validation,
                "设备请求无效。",
            ),
            DeviceAuthorizationFailureKind::InvalidAuthorization => (
                StatusCode::UNAUTHORIZED,
                "device.invalid_authorization",
                ErrorCategory::Authentication,
                "设备授权无效或已过期。",
            ),
            DeviceAuthorizationFailureKind::InvalidToken => (
                StatusCode::UNAUTHORIZED,
                "device.invalid_token",
                ErrorCategory::Authentication,
                "设备凭据无效或已过期。",
            ),
            DeviceAuthorizationFailureKind::InvalidProof => (
                StatusCode::UNAUTHORIZED,
                "device.invalid_proof",
                ErrorCategory::Authentication,
                "设备持有证明无效。",
            ),
            DeviceAuthorizationFailureKind::ProofReplay => (
                StatusCode::UNAUTHORIZED,
                "device.proof_replay",
                ErrorCategory::Authentication,
                "设备请求证明已被使用。",
            ),
            DeviceAuthorizationFailureKind::RefreshTokenReuse => (
                StatusCode::UNAUTHORIZED,
                "device.refresh_token_reuse",
                ErrorCategory::Authentication,
                "检测到刷新凭据重用，设备会话已撤销。",
            ),
            DeviceAuthorizationFailureKind::PrincipalSuspended => (
                StatusCode::FORBIDDEN,
                "device.principal_suspended",
                ErrorCategory::Authorization,
                "该主体已暂停。",
            ),
            DeviceAuthorizationFailureKind::DeviceRevoked => (
                StatusCode::FORBIDDEN,
                "device.revoked",
                ErrorCategory::Authorization,
                "该设备已撤销。",
            ),
            DeviceAuthorizationFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "device.not_found",
                ErrorCategory::Validation,
                "设备不存在。",
            ),
            DeviceAuthorizationFailureKind::Conflict => (
                StatusCode::CONFLICT,
                "device.conflict",
                ErrorCategory::Conflict,
                "设备状态发生冲突，请重新授权。",
            ),
            DeviceAuthorizationFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "device.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "设备认证依赖暂时不可用。",
            ),
            DeviceAuthorizationFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "device.internal",
                ErrorCategory::Transient,
                "设备认证服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            failure = ?failure.kind(),
            "设备认证请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn agent(failure: AgentManagementFailure, correlation_id: CorrelationId) -> Self {
        let (status, code, category, message) = match failure.kind() {
            AgentManagementFailureKind::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "agent.invalid_request",
                ErrorCategory::Validation,
                "Agent 请求无效。",
            ),
            AgentManagementFailureKind::Forbidden => (
                StatusCode::FORBIDDEN,
                "agent.forbidden",
                ErrorCategory::Authorization,
                "无权操作该 Agent。",
            ),
            AgentManagementFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "agent.not_found",
                ErrorCategory::Validation,
                "Agent 不存在。",
            ),
            AgentManagementFailureKind::Conflict => (
                StatusCode::CONFLICT,
                "agent.conflict",
                ErrorCategory::Conflict,
                "Agent 状态或幂等请求发生冲突。",
            ),
            AgentManagementFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "agent.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "Agent 身份依赖暂时不可用。",
            ),
            AgentManagementFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "agent.internal",
                ErrorCategory::Transient,
                "Agent 服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            failure = ?failure.kind(),
            "Agent 请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn agent_instance_management(
        failure: AgentInstanceManagementFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let (status, code, category, message) = match failure.kind() {
            AgentInstanceManagementFailureKind::Forbidden => (
                StatusCode::FORBIDDEN,
                "agent_instance.forbidden",
                ErrorCategory::Authorization,
                "无权管理该 Agent 实例。",
            ),
            AgentInstanceManagementFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "agent_instance.not_found",
                ErrorCategory::Validation,
                "Agent 实例不存在。",
            ),
            AgentInstanceManagementFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "agent_instance.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "Agent 实例管理依赖暂时不可用。",
            ),
            AgentInstanceManagementFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "agent_instance.internal",
                ErrorCategory::Transient,
                "Agent 实例管理服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            failure = ?failure.kind(),
            "Agent 实例管理请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn agent_lobby(
        failure: &AgentLobbyEntryFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let (status, code, category, message) = match failure.kind() {
            AgentLobbyEntryFailureKind::Unauthorized => (
                StatusCode::FORBIDDEN,
                "lobby.unauthorized",
                ErrorCategory::Authorization,
                "当前设备无权让该 Agent 实例进入大厅。",
            ),
            AgentLobbyEntryFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "lobby.not_found",
                ErrorCategory::Validation,
                "Agent 实例或大厅不存在。",
            ),
            AgentLobbyEntryFailureKind::Conflict => (
                StatusCode::CONFLICT,
                "lobby.conflict",
                ErrorCategory::Conflict,
                "大厅分配状态发生冲突，请重试。",
            ),
            AgentLobbyEntryFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "lobby.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "大厅依赖暂时不可用。",
            ),
            AgentLobbyEntryFailureKind::UnknownCommit => (
                StatusCode::SERVICE_UNAVAILABLE,
                "lobby.unknown_commit",
                ErrorCategory::UnknownCommit,
                "大厅操作提交状态未知，必须先对账。",
            ),
            AgentLobbyEntryFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "lobby.internal",
                ErrorCategory::Transient,
                "大厅服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            failure = ?failure,
            "Agent 大厅入口失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn private_room(failure: PrivateRoomFailure, correlation_id: CorrelationId) -> Self {
        let (status, code, category, message) = match failure.kind() {
            PrivateRoomFailureKind::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "private_room.invalid_request",
                ErrorCategory::Validation,
                "私人房间请求无效。",
            ),
            PrivateRoomFailureKind::Forbidden => (
                StatusCode::FORBIDDEN,
                "private_room.forbidden",
                ErrorCategory::Authorization,
                "无权访问或治理该私人房间。",
            ),
            PrivateRoomFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "private_room.not_found",
                ErrorCategory::Validation,
                "私人房间或目标主体不存在。",
            ),
            PrivateRoomFailureKind::Conflict => (
                StatusCode::CONFLICT,
                "private_room.conflict",
                ErrorCategory::Conflict,
                "私人房间状态已经变化，请刷新后重试。",
            ),
            PrivateRoomFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "private_room.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "私人房间依赖暂时不可用。",
            ),
            PrivateRoomFailureKind::UnknownCommit => (
                StatusCode::SERVICE_UNAVAILABLE,
                "private_room.unknown_commit",
                ErrorCategory::UnknownCommit,
                "私人房间操作提交状态未知，必须先对账。",
            ),
            PrivateRoomFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "private_room.internal",
                ErrorCategory::Transient,
                "私人房间服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            stage = ?failure.stage(),
            failure = ?failure.kind(),
            "私人房间请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn direct_session(
        failure: DirectSessionFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let (status, code, category, message) = match failure.kind() {
            DirectSessionFailureKind::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "direct_session.invalid_request",
                ErrorCategory::Validation,
                "直接会话请求无效。",
            ),
            DirectSessionFailureKind::Forbidden => (
                StatusCode::FORBIDDEN,
                "direct_session.forbidden",
                ErrorCategory::Authorization,
                "无权访问该直接会话。",
            ),
            DirectSessionFailureKind::Blocked => (
                StatusCode::FORBIDDEN,
                "direct_session.blocked",
                ErrorCategory::Authorization,
                "该联系人已被任一方屏蔽，不能建立或恢复投递。",
            ),
            DirectSessionFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "direct_session.not_found",
                ErrorCategory::Validation,
                "直接会话或目标 Agent 不存在。",
            ),
            DirectSessionFailureKind::Conflict => (
                StatusCode::CONFLICT,
                "direct_session.conflict",
                ErrorCategory::Conflict,
                "直接会话状态已经变化，请刷新后重试。",
            ),
            DirectSessionFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "direct_session.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "直接会话依赖暂时不可用。",
            ),
            DirectSessionFailureKind::UnknownCommit => (
                StatusCode::SERVICE_UNAVAILABLE,
                "direct_session.unknown_commit",
                ErrorCategory::UnknownCommit,
                "直接会话提交状态未知，必须先对账。",
            ),
            DirectSessionFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "direct_session.internal",
                ErrorCategory::Transient,
                "直接会话服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            stage = ?failure.stage(),
            failure = ?failure.kind(),
            "直接会话请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn agent_instance_verification(
        failure: AgentInstanceVerificationFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let (status, code, category, message) = match failure.kind() {
            AgentInstanceVerificationFailureKind::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "agent_instance_verification.unauthorized",
                ErrorCategory::Authentication,
                "设备授权无效或已过期。",
            ),
            AgentInstanceVerificationFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "agent_instance_verification.not_found",
                ErrorCategory::Validation,
                "Agent 实例不存在。",
            ),
            AgentInstanceVerificationFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "agent_instance_verification.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "Agent 实例验签材料暂时不可用。",
            ),
            AgentInstanceVerificationFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "agent_instance_verification.internal",
                ErrorCategory::Transient,
                "Agent 实例验签服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            failure = ?failure.kind(),
            "Agent 实例验签材料请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn handoff_access(
        failure: HandoffAccessFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let (status, code, category, message) = match failure.kind() {
            HandoffAccessFailureKind::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "handoff.unauthorized",
                ErrorCategory::Authentication,
                "设备授权无效或已过期。",
            ),
            HandoffAccessFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "handoff.instance_not_found",
                ErrorCategory::Validation,
                "Agent 实例不存在或当前主体无权访问。",
            ),
            HandoffAccessFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "handoff.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "交接授权依赖暂时不可用。",
            ),
            HandoffAccessFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "handoff.internal",
                ErrorCategory::Transient,
                "交接授权服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            failure = ?failure.kind(),
            "交接授权请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn targeted_handoff(
        failure: TargetedHandoffFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let (status, code, category, message) = match failure.kind() {
            TargetedHandoffFailureKind::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "targeted_handoff.unauthorized",
                ErrorCategory::Authentication,
                "当前人类会话或设备凭证无效。",
            ),
            TargetedHandoffFailureKind::Forbidden => (
                StatusCode::FORBIDDEN,
                "targeted_handoff.forbidden",
                ErrorCategory::Authorization,
                "当前主体无权访问该房间或交接任务。",
            ),
            TargetedHandoffFailureKind::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "targeted_handoff.invalid_request",
                ErrorCategory::Validation,
                "交接请求的标识、权限或有效期无效。",
            ),
            TargetedHandoffFailureKind::InvalidSource => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "targeted_handoff.invalid_source",
                ErrorCategory::Validation,
                "交接来源消息、事件或内容绑定不成立。",
            ),
            TargetedHandoffFailureKind::TargetUnavailable => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "targeted_handoff.target_unavailable",
                ErrorCategory::Validation,
                "目标 Agent 实例不存在、已撤销或不支持云端交接。",
            ),
            TargetedHandoffFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "targeted_handoff.not_found",
                ErrorCategory::Validation,
                "交接任务不存在或当前主体不可见。",
            ),
            TargetedHandoffFailureKind::Conflict => (
                StatusCode::CONFLICT,
                "targeted_handoff.conflict",
                ErrorCategory::Conflict,
                "交接任务状态或幂等请求已经发生冲突。",
            ),
            TargetedHandoffFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "targeted_handoff.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "交接队列依赖暂时不可用。",
            ),
            TargetedHandoffFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "targeted_handoff.internal",
                ErrorCategory::Transient,
                "交接队列发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            failure = ?failure.kind(),
            "定向交接请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn agent_card(
        failure: AgentCardManagementFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let (status, code, category, message) = match failure.kind() {
            AgentCardManagementFailureKind::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "agent_card.invalid_request",
                ErrorCategory::Validation,
                "Agent Card 请求或文档无效。",
            ),
            AgentCardManagementFailureKind::Forbidden => (
                StatusCode::FORBIDDEN,
                "agent_card.forbidden",
                ErrorCategory::Authorization,
                "无权刷新该 Agent Card。",
            ),
            AgentCardManagementFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "agent_card.not_found",
                ErrorCategory::Validation,
                "Agent 不存在。",
            ),
            AgentCardManagementFailureKind::UntrustedSource => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "agent_card.untrusted_source",
                ErrorCategory::Validation,
                "Agent Card 来源或签名无法信任。",
            ),
            AgentCardManagementFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "agent_card.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "Agent Card 依赖暂时不可用。",
            ),
            AgentCardManagementFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "agent_card.internal",
                ErrorCategory::Transient,
                "Agent Card 服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            failure = ?failure.kind(),
            "Agent Card 请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn begin_content_upload(
        failure: &BeginContentUploadFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let mapping = match failure {
            BeginContentUploadFailure::Denied => authorization_denied("content.upload.denied"),
            BeginContentUploadFailure::Authorization(error) => {
                content_authorization_mapping(error, "content.upload")
            }
            BeginContentUploadFailure::Domain(_) => invalid_content("content.upload.invalid"),
            BeginContentUploadFailure::StorageKey(_) => internal_content("content.upload.internal"),
            BeginContentUploadFailure::Repository(error) => {
                repository_mapping(error, "content.upload")
            }
        };
        log_content_failure("begin_upload", failure, correlation_id);
        from_mapping(mapping, correlation_id)
    }

    pub(crate) fn complete_content_upload(
        failure: &CompleteContentUploadFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let mapping = match failure {
            CompleteContentUploadFailure::NotFound => content_not_found(),
            CompleteContentUploadFailure::Forbidden => {
                authorization_denied("content.upload.forbidden")
            }
            CompleteContentUploadFailure::InvalidState(_) => {
                content_conflict("content.upload.invalid_state")
            }
            CompleteContentUploadFailure::Repository { error, .. } => {
                repository_mapping(error, "content.upload")
            }
            CompleteContentUploadFailure::ObjectStore { error, .. } => {
                object_store_mapping(error.kind(), "content.upload")
            }
            CompleteContentUploadFailure::Scan(error) => match error.kind() {
                ContentScanFailureKind::Unavailable | ContentScanFailureKind::InvalidResponse => {
                    dependency_content("content.scan.unavailable")
                }
            },
            CompleteContentUploadFailure::IntegrityMismatch { .. } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "content.upload.integrity_mismatch",
                ErrorCategory::Validation,
                "上传正文与声明的摘要或长度不一致。",
            ),
            CompleteContentUploadFailure::ScanRejected { .. } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "content.upload.rejected",
                ErrorCategory::Validation,
                "上传内容未通过安全扫描。",
            ),
        };
        log_content_failure("complete_upload", failure, correlation_id);
        from_mapping(mapping, correlation_id)
    }

    pub(crate) fn bind_content_event(
        failure: &BindContentEventFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let mapping = match failure {
            BindContentEventFailure::NotFound => content_not_found(),
            BindContentEventFailure::Forbidden | BindContentEventFailure::Revoked => {
                authorization_denied("content.binding.forbidden")
            }
            BindContentEventFailure::InvalidState(_)
            | BindContentEventFailure::PolicyMismatch
            | BindContentEventFailure::EventConflict => {
                content_conflict("content.binding.conflict")
            }
            BindContentEventFailure::Repository(error) => {
                repository_mapping(error, "content.binding")
            }
        };
        log_content_failure("bind_event", failure, correlation_id);
        from_mapping(mapping, correlation_id)
    }

    pub(crate) fn redact_content(
        failure: &RedactContentFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let mapping = match failure {
            RedactContentFailure::NotFound => content_not_found(),
            RedactContentFailure::Forbidden => authorization_denied("content.redact.forbidden"),
            RedactContentFailure::InvalidState(_) => {
                content_conflict("content.redact.invalid_state")
            }
            RedactContentFailure::Repository(error) => repository_mapping(error, "content.redact"),
        };
        log_content_failure("redact", failure, correlation_id);
        from_mapping(mapping, correlation_id)
    }

    pub(crate) fn issue_content_ticket(
        failure: &IssueContentReadTicketFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let mapping = match failure {
            IssueContentReadTicketFailure::NotFound => content_not_found(),
            IssueContentReadTicketFailure::NotReadable(_) => {
                content_conflict("content.read.not_readable")
            }
            IssueContentReadTicketFailure::EventNotBound => {
                content_conflict("content.read.event_not_bound")
            }
            IssueContentReadTicketFailure::Denied => authorization_denied("content.read.denied"),
            IssueContentReadTicketFailure::Domain(_) => invalid_content("content.read.invalid"),
            IssueContentReadTicketFailure::Repository(error) => {
                repository_mapping(error, "content.read")
            }
            IssueContentReadTicketFailure::Authorization(error) => {
                content_authorization_mapping(error, "content.read")
            }
            IssueContentReadTicketFailure::Ticket(error) => ticket_mapping(error, "content.read"),
        };
        log_content_failure("issue_read_ticket", failure, correlation_id);
        from_mapping(mapping, correlation_id)
    }

    pub(crate) fn open_content(
        failure: &OpenContentFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let mapping = match failure {
            OpenContentFailure::Ticket(error) => ticket_mapping(error, "content.open"),
            OpenContentFailure::NotFound => content_not_found(),
            OpenContentFailure::NotReadable(state) => unreadable_mapping(*state),
            OpenContentFailure::StaleTicket => content_conflict("content.open.stale_ticket"),
            OpenContentFailure::Denied => authorization_denied("content.open.denied"),
            OpenContentFailure::Repository(error) => repository_mapping(error, "content.open"),
            OpenContentFailure::Authorization(error) => {
                content_authorization_mapping(error, "content.open")
            }
            OpenContentFailure::RateLimit(_) => {
                dependency_content("content.rate_limit.unavailable")
            }
            OpenContentFailure::RateLimited { retry_at } => {
                let error = Self::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "content.rate_limited",
                    ErrorCategory::Transient,
                    "内容读取过于频繁，请稍后重试。",
                    correlation_id,
                )
                .retry_after_seconds(seconds_until(*retry_at));
                log_content_failure("open", failure, correlation_id);
                return error;
            }
            OpenContentFailure::ObjectStore(error) => {
                object_store_mapping(error.kind(), "content.open")
            }
            OpenContentFailure::ObjectMetadataMismatch => {
                dependency_content("content.open.corrupt_metadata")
            }
        };
        log_content_failure("open", failure, correlation_id);
        from_mapping(mapping, correlation_id)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let retry_after_seconds = self.envelope.retry_after_seconds;
        let mut response = (self.status, Json(self.envelope)).into_response();
        if let Some(seconds) = retry_after_seconds
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
    }
}

type ErrorMapping = (StatusCode, &'static str, ErrorCategory, &'static str);

fn from_mapping(mapping: ErrorMapping, correlation_id: CorrelationId) -> ApiError {
    ApiError::new(mapping.0, mapping.1, mapping.2, mapping.3, correlation_id)
}

fn invalid_content(code: &'static str) -> ErrorMapping {
    (
        StatusCode::BAD_REQUEST,
        code,
        ErrorCategory::Validation,
        "内容请求无效。",
    )
}

fn authorization_denied(code: &'static str) -> ErrorMapping {
    (
        StatusCode::FORBIDDEN,
        code,
        ErrorCategory::Authorization,
        "当前主体无权执行该内容操作。",
    )
}

fn content_not_found() -> ErrorMapping {
    (
        StatusCode::NOT_FOUND,
        "content.not_found",
        ErrorCategory::Validation,
        "内容不存在。",
    )
}

fn content_conflict(code: &'static str) -> ErrorMapping {
    (
        StatusCode::CONFLICT,
        code,
        ErrorCategory::Conflict,
        "内容状态已经变化，请刷新后重试。",
    )
}

fn dependency_content(code: &'static str) -> ErrorMapping {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        code,
        ErrorCategory::DependencyUnavailable,
        "内容依赖暂时不可用。",
    )
}

fn internal_content(code: &'static str) -> ErrorMapping {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        code,
        ErrorCategory::Transient,
        "内容服务发生内部错误。",
    )
}

fn repository_mapping(error: &RepositoryError, prefix: &'static str) -> ErrorMapping {
    match error.kind() {
        RepositoryErrorKind::Conflict => content_conflict(match prefix {
            "content.upload" => "content.upload.idempotency_conflict",
            "content.binding" => "content.binding.conflict",
            _ => "content.repository.conflict",
        }),
        RepositoryErrorKind::Forbidden => authorization_denied("content.repository.forbidden"),
        RepositoryErrorKind::NotFound => content_not_found(),
        RepositoryErrorKind::Unavailable => dependency_content("content.repository.unavailable"),
        RepositoryErrorKind::Constraint | RepositoryErrorKind::CorruptData => {
            internal_content("content.repository.internal")
        }
    }
}

fn content_authorization_mapping(
    error: &ContentAuthorizationFailure,
    prefix: &'static str,
) -> ErrorMapping {
    match error.kind() {
        ContentAuthorizationFailureKind::Denied => authorization_denied(match prefix {
            "content.upload" => "content.upload.denied",
            "content.open" => "content.open.denied",
            _ => "content.read.denied",
        }),
        ContentAuthorizationFailureKind::StaleProjection
        | ContentAuthorizationFailureKind::Unavailable => {
            dependency_content("content.authorization.unavailable")
        }
    }
}

fn ticket_mapping(error: &ContentTicketFailure, prefix: &'static str) -> ErrorMapping {
    match error.kind() {
        ContentTicketFailureKind::Invalid | ContentTicketFailureKind::Expired => (
            StatusCode::UNAUTHORIZED,
            if prefix == "content.open" {
                "content.open.invalid_ticket"
            } else {
                "content.read.ticket_failure"
            },
            ErrorCategory::Authentication,
            "内容读取票据无效或已过期。",
        ),
        ContentTicketFailureKind::AudienceMismatch => {
            authorization_denied("content.open.ticket_audience_mismatch")
        }
        ContentTicketFailureKind::Unavailable => dependency_content("content.ticket.unavailable"),
    }
}

fn object_store_mapping(kind: ObjectStoreFailureKind, prefix: &'static str) -> ErrorMapping {
    match kind {
        ObjectStoreFailureKind::Rejected => invalid_content(match prefix {
            "content.upload" => "content.upload.object_rejected",
            _ => "content.open.object_rejected",
        }),
        ObjectStoreFailureKind::NotFound
        | ObjectStoreFailureKind::CorruptMetadata
        | ObjectStoreFailureKind::Unavailable => {
            dependency_content("content.object_store.unavailable")
        }
    }
}

fn unreadable_mapping(state: ContentLifecycleState) -> ErrorMapping {
    match state {
        ContentLifecycleState::Uploading | ContentLifecycleState::Active => {
            content_conflict("content.open.not_readable")
        }
        ContentLifecycleState::Orphaned
        | ContentLifecycleState::Redacted
        | ContentLifecycleState::Expired
        | ContentLifecycleState::Deleted => (
            StatusCode::GONE,
            "content.open.gone",
            ErrorCategory::Conflict,
            "内容已不可读取。",
        ),
    }
}

fn seconds_until(retry_at: agent_room_domain::time::UtcMillis) -> u64 {
    let now_millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_i128, |duration| {
            i128::try_from(duration.as_millis()).unwrap_or(i128::MAX)
        });
    let remaining = i128::from(retry_at.value())
        .saturating_sub(now_millis)
        .max(1);
    u64::try_from((remaining.saturating_add(999)) / 1_000).unwrap_or(u64::MAX)
}

fn log_content_failure(
    operation: &'static str,
    failure: &impl std::fmt::Debug,
    correlation_id: CorrelationId,
) {
    tracing::warn!(
        correlation.id = %correlation_id.as_uuid(),
        operation,
        failure = ?failure,
        "内容请求失败"
    );
}

fn log_moderation_failure(failure: ModerationFailure, correlation_id: CorrelationId) {
    tracing::warn!(
        correlation.id = %correlation_id.as_uuid(),
        operation = failure.operation(),
        failure = ?failure.kind(),
        "治理请求失败"
    );
}

pub(crate) async fn not_found(Extension(correlation_id): Extension<CorrelationId>) -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "http.route_not_found",
        ErrorCategory::Validation,
        "请求的资源不存在。",
        correlation_id,
    )
}

pub(crate) async fn method_not_allowed(
    Extension(correlation_id): Extension<CorrelationId>,
) -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "http.method_not_allowed",
        ErrorCategory::Validation,
        "该资源不支持当前请求方法。",
        correlation_id,
    )
}
