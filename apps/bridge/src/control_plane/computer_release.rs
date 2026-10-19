//! 换账号时把这台电脑移出原来的账户：`POST /auth/devices/current/release`，用这台电脑的设备签名，
//! 正文为空（`specs/sign-out/design.md` 第 4 步）。控制面照“撤销电脑”那样撤掉它和它的 Agent 连接。

use std::sync::Arc;

use agent_room_bridge_core::session::{BridgeSessionFailureKind, ControlPlaneRequestAuthorizer};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use url::Url;

use super::{
    ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError, configured_client,
    read_limited_response_body, signed_request_headers,
};

const RELEASE_TARGET: &str = "/auth/devices/current/release";

/// 控制面暂时不可用（连不上、5xx、429、设备会话暂时刷新不了），过一会儿可以再试。
pub const RELEASE_UNAVAILABLE: &str = "bridge.release_unavailable";
/// 控制面拒绝了（不是凭据作废），再试也一样。
pub const RELEASE_REJECTED: &str = "bridge.release_rejected";
/// 控制面还没有这个接口。
pub const RELEASE_UNSUPPORTED: &str = "bridge.release_unsupported";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputerReleaseOutcome {
    /// 这次移出去了。
    Released,
    /// 这台电脑的凭据早就作废（在别处移除了、账户停用了）：服务器上已经不算这个账户的电脑。
    AlreadyReleased,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComputerReleaseFailure {
    code: &'static str,
}

impl ComputerReleaseFailure {
    pub const fn code(self) -> &'static str {
        self.code
    }
}

pub type ComputerReleaseResult = Result<ComputerReleaseOutcome, ComputerReleaseFailure>;

pub struct ReqwestComputerRelease {
    client: Client,
    url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

impl ReqwestComputerRelease {
    /// 创建以这台电脑的设备会话签名、只向固定控制面发请求的客户端。
    ///
    /// # Errors
    ///
    /// 控制面地址、明文传输边界、超时或 HTTP 客户端配置无效时返回错误。
    pub fn new(
        config: &ControlPlaneHttpConfig,
        authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
    ) -> Result<Self, ControlPlaneHttpConfigurationError> {
        let (client, base_url) = configured_client(config)?;
        let url = base_url
            .join(RELEASE_TARGET.trim_start_matches('/'))
            .map_err(|_| ControlPlaneHttpConfigurationError::InvalidBaseUrl)?;
        Ok(Self {
            client,
            url,
            authorizer,
        })
    }

    /// 请控制面把这台电脑移出它现在所属的账户。可以放心重试：已经移出去的再请一次，凭据已经作废，
    /// 算 [`ComputerReleaseOutcome::AlreadyReleased`]。
    ///
    /// # Errors
    ///
    /// 控制面暂时不可用或拒绝时返回带稳定错误码的失败，这时服务器上什么都没变。
    pub async fn release(&self) -> ComputerReleaseResult {
        let authorized = match self.authorizer.authorize("POST", RELEASE_TARGET, "").await {
            Ok(authorized) => authorized,
            Err(failure) => return outcome_for_session_failure(failure.kind()),
        };
        let request = signed_request_headers(
            self.client.post(self.url.clone()),
            &authorized,
            "POST",
            RELEASE_TARGET,
        )
        .map_err(|()| failure(RELEASE_REJECTED))?;
        let response = request
            .body(Vec::new())
            .send()
            .await
            .map_err(|_| failure(RELEASE_UNAVAILABLE))?;
        let status = response.status();
        if status.is_success() {
            // 204 是都撤完了；202 是本地撤完、Matrix 那边还在清理，这台电脑同样已经不属于这个账户。
            return Ok(ComputerReleaseOutcome::Released);
        }
        let code = read_limited_response_body(response)
            .await
            .ok()
            .and_then(|body| serde_json::from_slice::<ErrorCodeResponse>(&body).ok())
            .map(|envelope| envelope.code);
        outcome_for_status(status, code.as_deref())
    }
}

#[derive(Deserialize)]
struct ErrorCodeResponse {
    code: String,
}

/// 签名之前就失败：设备会话已经作废说明服务器上早就不算这台电脑了；别的都算暂时不可用。
fn outcome_for_session_failure(kind: BridgeSessionFailureKind) -> ComputerReleaseResult {
    match kind {
        BridgeSessionFailureKind::NotAuthorized => Ok(ComputerReleaseOutcome::AlreadyReleased),
        BridgeSessionFailureKind::RefreshOutcomeUnknown
        | BridgeSessionFailureKind::ControlPlaneUnavailable
        | BridgeSessionFailureKind::SecureStorageUnavailable => Err(failure(RELEASE_UNAVAILABLE)),
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => Err(failure(RELEASE_REJECTED)),
    }
}

/// 只有凭据确实作废才算已经移出去；签名不对、重放这类 401 是这次请求的问题，服务器上还算这台电脑。
fn outcome_for_status(status: StatusCode, code: Option<&str>) -> ComputerReleaseResult {
    match (status, code) {
        (StatusCode::UNAUTHORIZED, Some("device.invalid_token" | "device.refresh_token_reuse"))
        | (StatusCode::FORBIDDEN, Some("device.revoked" | "device.principal_suspended")) => {
            Ok(ComputerReleaseOutcome::AlreadyReleased)
        }
        (StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED, _) => {
            Err(failure(RELEASE_UNSUPPORTED))
        }
        (StatusCode::TOO_MANY_REQUESTS, _) => Err(failure(RELEASE_UNAVAILABLE)),
        (status, _) if status.is_server_error() => Err(failure(RELEASE_UNAVAILABLE)),
        _ => Err(failure(RELEASE_REJECTED)),
    }
}

const fn failure(code: &'static str) -> ComputerReleaseFailure {
    ComputerReleaseFailure { code }
}

#[cfg(test)]
mod tests;
