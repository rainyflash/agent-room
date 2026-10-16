//! 桌面端的聊天登录由控制面替人发（`specs/desktop-sign-in/design.md`）：以应用服务的身份替这个本地
//! 的人登录 Matrix，拿一台带刷新令牌的新设备。还没有 Matrix 账户（第一次用就是桌面端）时先注册，
//! 用户名就是控制面记着的那个，和 Synapse 的登录映射器同一条规则；之后这个人在网页上走 SSO，
//! Synapse 靠 `allow_existing_users` 认下这个账户。
//!
//! 这要求应用服务注册里有一个覆盖本地用户的非独占命名空间（`tools/prodops/render.py`，ADR 0011）。

use agent_room_application::ports::{
    MatrixAgentLocalpart, MatrixDeviceId, MatrixFailure, MatrixFailureKind,
    MatrixHumanSessionIssuer, MatrixOperation, MatrixResult, MatrixSession, MatrixSessionMetadata,
    MatrixUserId, PortFuture, SecretValue,
};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::{
    ApplicationServiceRegistrationRequest, DeviceSessionResponse,
    MatrixApplicationServiceProvisioner, MatrixUserIdentifier, decode_json, decode_matrix_error,
    invalid_response, map_matrix_error, map_transport_error, read_limited_body,
    rooms::endpoint_with_segments,
};

/// 桌面端登录出来的设备在设备列表里叫这个，和以前走 SSO 时网页层填的一样。
const DESKTOP_DEVICE_DISPLAY_NAME: &str = "Agent Room Desktop";

#[derive(Serialize)]
struct HumanLoginRequest<'a> {
    #[serde(rename = "type")]
    login_type: &'static str,
    identifier: MatrixUserIdentifier<'a>,
    initial_device_display_name: &'static str,
    refresh_token: bool,
}

#[derive(Serialize, Deserialize)]
struct DisplayName {
    #[serde(default)]
    displayname: Option<String>,
}

impl MatrixApplicationServiceProvisioner {
    async fn issue_human_session_internal(
        &self,
        user_id: &MatrixUserId,
        display_name: &str,
    ) -> MatrixResult<MatrixSession> {
        let operation = MatrixOperation::IssueHumanSession;
        let localpart = self.human_localpart(user_id, operation)?;
        self.register_human(user_id, localpart, operation).await?;
        self.ensure_display_name(user_id, localpart, display_name, operation)
            .await?;
        let response = self
            .client
            .post(self.endpoint("_matrix/client/v3/login", operation)?)
            .bearer_auth(self.access_token.expose())
            .json(&HumanLoginRequest {
                login_type: "m.login.application_service",
                identifier: MatrixUserIdentifier {
                    identifier_type: "m.id.user",
                    user: user_id.as_str(),
                },
                initial_device_display_name: DESKTOP_DEVICE_DISPLAY_NAME,
                refresh_token: true,
            })
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_limited_body(response, operation).await?;
        if !status.is_success() {
            let error = decode_matrix_error(&body, operation)?;
            return Err(map_matrix_error(operation, status, &error));
        }
        let issued: DeviceSessionResponse = decode_json(&body, operation)?;
        if issued.user_id != user_id.as_str() {
            return Err(invalid_response(operation));
        }
        let device_id =
            MatrixDeviceId::new(issued.device_id).map_err(|_| invalid_response(operation))?;
        let access_token =
            SecretValue::new(issued.access_token).map_err(|_| invalid_response(operation))?;
        let refresh_token = issued
            .refresh_token
            .map(SecretValue::new)
            .transpose()
            .map_err(|_| invalid_response(operation))?;
        Ok(MatrixSession::new(
            MatrixSessionMetadata::new(user_id.clone(), device_id),
            access_token,
            refresh_token,
        ))
    }

    /// 只替本服务器上的人登录。Agent 的用户由 Agent 那一套发，这里不碰。
    fn human_localpart<'a>(
        &self,
        user_id: &'a MatrixUserId,
        operation: MatrixOperation,
    ) -> MatrixResult<&'a str> {
        let suffix = format!(":{}", self.server_name);
        let localpart = user_id
            .as_str()
            .strip_prefix('@')
            .and_then(|value| value.strip_suffix(suffix.as_str()))
            .filter(|localpart| !localpart.is_empty())
            .ok_or_else(|| MatrixFailure::new(operation, MatrixFailureKind::Forbidden))?;
        if MatrixAgentLocalpart::new(localpart).is_ok() {
            return Err(MatrixFailure::new(operation, MatrixFailureKind::Forbidden));
        }
        Ok(localpart)
    }

    /// 还没有账户就注册；已经有了（以前走 SSO 建的，或者上一次注册过）也算成。
    async fn register_human(
        &self,
        user_id: &MatrixUserId,
        localpart: &str,
        operation: MatrixOperation,
    ) -> MatrixResult<()> {
        let response = self
            .client
            .post(self.endpoint("_matrix/client/v3/register", operation)?)
            .bearer_auth(self.access_token.expose())
            .json(&ApplicationServiceRegistrationRequest {
                login_type: "m.login.application_service",
                username: localpart,
                inhibit_login: true,
            })
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_limited_body(response, operation).await?;
        if status.is_success() {
            let registered: crate::UserRegistrationResponse = decode_json(&body, operation)?;
            if registered.user_id != user_id.as_str() {
                return Err(invalid_response(operation));
            }
            return Ok(());
        }
        let error = decode_matrix_error(&body, operation)?;
        if status == StatusCode::BAD_REQUEST && error.errcode == "M_USER_IN_USE" {
            return Ok(());
        }
        Err(map_matrix_error(operation, status, &error))
    }

    /// 应用服务注册的账户，Synapse 把显示名设成用户名。还是那样（或者没有）就写成账户的名字；
    /// 已经有别的名字（走 SSO 时从身份服务带来的、本人改过的）不动。注册以后没写成的，下次再写。
    async fn ensure_display_name(
        &self,
        user_id: &MatrixUserId,
        localpart: &str,
        display_name: &str,
        operation: MatrixOperation,
    ) -> MatrixResult<()> {
        let mut endpoint = endpoint_with_segments(
            &self.homeserver_url,
            &[
                "_matrix",
                "client",
                "v3",
                "profile",
                user_id.as_str(),
                "displayname",
            ],
            operation,
        )?;
        endpoint
            .query_pairs_mut()
            .append_pair("user_id", user_id.as_str());
        let response = self
            .client
            .get(endpoint.clone())
            .bearer_auth(self.access_token.expose())
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_limited_body(response, operation).await?;
        let current = if status.is_success() {
            decode_json::<DisplayName>(&body, operation)?.displayname
        } else if status == StatusCode::NOT_FOUND {
            None
        } else {
            let error = decode_matrix_error(&body, operation)?;
            return Err(map_matrix_error(operation, status, &error));
        };
        if current.as_deref().is_some_and(|name| name != localpart) {
            return Ok(());
        }
        let response = self
            .client
            .put(endpoint)
            .bearer_auth(self.access_token.expose())
            .json(&DisplayName {
                displayname: Some(display_name.to_owned()),
            })
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_limited_body(response, operation).await?;
        if status.is_success() {
            return Ok(());
        }
        let error = decode_matrix_error(&body, operation)?;
        Err(map_matrix_error(operation, status, &error))
    }
}

impl MatrixHumanSessionIssuer for MatrixApplicationServiceProvisioner {
    fn issue_human_session<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
        display_name: &'a str,
    ) -> PortFuture<'a, MatrixResult<MatrixSession>> {
        Box::pin(self.issue_human_session_internal(user_id, display_name))
    }
}

#[cfg(test)]
#[path = "human_sessions/tests.rs"]
mod tests;
