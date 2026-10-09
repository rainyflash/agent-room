//! 删除账户时删掉 Keycloak 里的登录账户：邮箱、昵称和密码散列都存在那里。
//!
//! 控制面用一个只开了服务账号的客户端（realm-management 的 `manage-users`）先拿客户端凭据令牌，
//! 再调管理接口删用户。只删本部署签发方的账户；用户已经不在也算成功，任务重试时会再调一次。

use std::time::Duration;

use agent_room_application::ports::{
    OidcFailure, OidcFailureKind, OidcResult, PortFuture, SecretValue, SignInAccount,
    SignInAccountRemoval,
};
use openidconnect::{
    ClientId, ClientSecret, IssuerUrl, JsonWebKeySet, OAuth2TokenResponse, TokenUrl,
    core::CoreClient,
    reqwest::{self, StatusCode},
    url::Url,
};
use thiserror::Error;

use crate::map_token_request_error;

/// Keycloak 用户号（默认是 UUID）的长度上限；超过就当成坏数据，不拿去拼地址。
const MAX_SUBJECT_LENGTH: usize = 255;

pub struct KeycloakAccountRemovalConfig {
    /// 控制面到 Keycloak 的内部地址，比如 `http://identity:8080`。令牌和管理接口都走它。
    pub internal_url: String,
    /// 登录用的公开签发方，比如 `https://auth.example/realms/agent-room`。领域名取它最后一段，
    /// 只删签发方和它一样的账户。
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: SecretValue,
    pub request_timeout: Duration,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum KeycloakAccountRemovalConfigError {
    #[error("Keycloak 内部地址无效")]
    InvalidInternalUrl,
    #[error("签发方地址不是 Keycloak 领域地址")]
    InvalidIssuer,
    #[error("删除账户用的客户端标识或超时无效")]
    InvalidClient,
    #[error("无法创建访问 Keycloak 的 HTTP 客户端")]
    HttpClient,
}

pub struct KeycloakSignInAccountRemoval {
    issuer: IssuerUrl,
    token_url: TokenUrl,
    users_url: Url,
    client_id: ClientId,
    client_secret: ClientSecret,
    http_client: reqwest::Client,
}

impl KeycloakSignInAccountRemoval {
    /// 只校验本地配置，不发网络请求。
    ///
    /// # Errors
    ///
    /// 地址、领域名、客户端标识、超时或 HTTP 客户端无效时返回配置错误。
    pub fn new(
        config: KeycloakAccountRemovalConfig,
    ) -> Result<Self, KeycloakAccountRemovalConfigError> {
        if config.client_id.is_empty()
            || config.client_id.len() > 255
            || config.client_id.chars().any(char::is_control)
            || config.request_timeout.is_zero()
        {
            return Err(KeycloakAccountRemovalConfigError::InvalidClient);
        }
        let realm = realm_of(&config.issuer_url)?;
        let issuer = IssuerUrl::new(config.issuer_url)
            .map_err(|_| KeycloakAccountRemovalConfigError::InvalidIssuer)?;
        let internal = Url::parse(config.internal_url.trim_end_matches('/'))
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())
            .filter(|url| url.query().is_none() && url.fragment().is_none())
            .ok_or(KeycloakAccountRemovalConfigError::InvalidInternalUrl)?;
        let token_url = TokenUrl::from_url(extend(
            &internal,
            &["realms", &realm, "protocol", "openid-connect", "token"],
        )?);
        let users_url = extend(&internal, &["admin", "realms", &realm, "users"])?;
        let http_client = reqwest::ClientBuilder::new()
            .timeout(config.request_timeout)
            .connect_timeout(config.request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| KeycloakAccountRemovalConfigError::HttpClient)?;
        Ok(Self {
            issuer,
            token_url,
            users_url,
            client_id: ClientId::new(config.client_id),
            client_secret: ClientSecret::new(config.client_secret.expose().to_owned()),
            http_client,
        })
    }

    fn is_ours(&self, account: &SignInAccount) -> bool {
        account.issuer.trim_end_matches('/') == self.issuer.as_str().trim_end_matches('/')
    }

    async fn access_token(&self) -> OidcResult<String> {
        let client = CoreClient::new(
            self.client_id.clone(),
            self.issuer.clone(),
            JsonWebKeySet::default(),
        )
        .set_client_secret(self.client_secret.clone())
        .set_token_uri(self.token_url.clone());
        let response = client
            .exchange_client_credentials()
            .request_async(&self.http_client)
            .await
            .map_err(|error| match map_token_request_error(&error).kind() {
                // 令牌响应解析不了多半是没连到真正的 Keycloak，下次再试。
                OidcFailureKind::InvalidIdentityToken => {
                    OidcFailure::new(OidcFailureKind::DependencyUnavailable)
                }
                kind => OidcFailure::new(kind),
            })?;
        Ok(response.access_token().secret().clone())
    }

    async fn delete_user(&self, subject: &str) -> OidcResult<()> {
        if subject.is_empty()
            || subject.len() > MAX_SUBJECT_LENGTH
            || subject.chars().any(char::is_control)
            || matches!(subject, "." | "..")
        {
            return Err(OidcFailure::new(OidcFailureKind::InvalidConfiguration));
        }
        let token = self.access_token().await?;
        let mut url = self.users_url.clone();
        url.path_segments_mut()
            .map_err(|()| OidcFailure::new(OidcFailureKind::InvalidConfiguration))?
            .push(subject);
        let response = self
            .http_client
            .delete(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| OidcFailure::new(OidcFailureKind::DependencyUnavailable))?;
        removal_outcome(response.status())
    }
}

impl SignInAccountRemoval for KeycloakSignInAccountRemoval {
    fn remove<'a>(&'a self, account: &'a SignInAccount) -> PortFuture<'a, OidcResult<()>> {
        Box::pin(async move {
            if !self.is_ours(account) {
                return Ok(());
            }
            self.delete_user(&account.subject).await
        })
    }
}

fn removal_outcome(status: StatusCode) -> OidcResult<()> {
    if status.is_success() || status == StatusCode::NOT_FOUND {
        return Ok(());
    }
    let kind = if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        OidcFailureKind::DependencyUnavailable
    } else {
        OidcFailureKind::ProviderRejected
    };
    Err(OidcFailure::new(kind))
}

fn realm_of(issuer_url: &str) -> Result<String, KeycloakAccountRemovalConfigError> {
    let issuer =
        Url::parse(issuer_url).map_err(|_| KeycloakAccountRemovalConfigError::InvalidIssuer)?;
    let segments: Vec<&str> = issuer
        .path_segments()
        .map(|segments| segments.filter(|segment| !segment.is_empty()).collect())
        .unwrap_or_default();
    match segments.as_slice() {
        [.., "realms", realm] if !realm.is_empty() => Ok((*realm).to_owned()),
        _ => Err(KeycloakAccountRemovalConfigError::InvalidIssuer),
    }
}

fn extend(base: &Url, segments: &[&str]) -> Result<Url, KeycloakAccountRemovalConfigError> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|()| KeycloakAccountRemovalConfigError::InvalidInternalUrl)?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 领域名取签发方地址最后一段() {
        assert_eq!(
            realm_of("https://auth.example/realms/agent-room").as_deref(),
            Ok("agent-room")
        );
        assert_eq!(
            realm_of("https://auth.example/realms/agent-room/").as_deref(),
            Ok("agent-room")
        );
        assert_eq!(
            realm_of("https://auth.example/agent-room"),
            Err(KeycloakAccountRemovalConfigError::InvalidIssuer)
        );
    }

    #[test]
    fn 用户已经不在也算删掉了_服务端出错下次再试() {
        assert_eq!(removal_outcome(StatusCode::NO_CONTENT), Ok(()));
        assert_eq!(removal_outcome(StatusCode::NOT_FOUND), Ok(()));
        assert_eq!(
            removal_outcome(StatusCode::FORBIDDEN).map_err(OidcFailure::kind),
            Err(OidcFailureKind::ProviderRejected)
        );
        assert_eq!(
            removal_outcome(StatusCode::SERVICE_UNAVAILABLE).map_err(OidcFailure::kind),
            Err(OidcFailureKind::DependencyUnavailable)
        );
    }
}
