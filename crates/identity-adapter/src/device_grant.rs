use std::{str::FromStr, time::Duration};

use agent_room_application::ports::{
    OidcDeviceAssertionVerifier, OidcDeviceAuthorizationPrompt, OidcDeviceAuthorizationPromptSink,
    OidcDeviceGrantGateway, OidcFailure, OidcFailureKind, OidcResult, PortFuture, SecretValue,
    VerifiedOidcIdentity,
};
use agent_room_domain::time::{DurationMillis, UtcMillis};
use openidconnect::{
    AdditionalProviderMetadata, AuthType, ClientId, DeviceAuthorizationUrl, IssuerUrl, Nonce,
    ProviderMetadata, Scope,
    core::{
        CoreAuthDisplay, CoreClaimName, CoreClaimType, CoreClient, CoreClientAuthMethod,
        CoreDeviceAuthorizationResponse, CoreGrantType, CoreIdToken, CoreJsonWebKey,
        CoreJweContentEncryptionAlgorithm, CoreJweKeyManagementAlgorithm, CoreResponseMode,
        CoreResponseType, CoreSubjectIdentifierType,
    },
    reqwest,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::OnceCell;

use crate::{invalid_identity_token, map_discovery_error, map_token_request_error};

const MINIMUM_POLLING_DURATION: Duration = Duration::from_secs(5);
const MAXIMUM_POLLING_DURATION: Duration = Duration::from_mins(30);

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DeviceEndpointProviderMetadata {
    device_authorization_endpoint: DeviceAuthorizationUrl,
}

impl AdditionalProviderMetadata for DeviceEndpointProviderMetadata {}

type DeviceProviderMetadata = ProviderMetadata<
    DeviceEndpointProviderMetadata,
    CoreAuthDisplay,
    CoreClientAuthMethod,
    CoreClaimName,
    CoreClaimType,
    CoreGrantType,
    CoreJweContentEncryptionAlgorithm,
    CoreJweKeyManagementAlgorithm,
    CoreJsonWebKey,
    CoreResponseMode,
    CoreResponseType,
    CoreSubjectIdentifierType,
>;

pub struct OidcDeviceGrantConfig {
    pub issuer_url: String,
    pub client_id: String,
    pub request_timeout: Duration,
    pub maximum_polling_duration: Duration,
}

/// 基于 OIDC Discovery 的 RFC 8628 公共客户端适配器。
pub struct DiscoveredOidcDeviceGrant {
    issuer: IssuerUrl,
    client_id: ClientId,
    request_timeout: Duration,
    maximum_polling_duration: Duration,
    http_client: reqwest::Client,
    metadata: OnceCell<DeviceProviderMetadata>,
}

impl DiscoveredOidcDeviceGrant {
    /// 创建惰性设备授权适配器，构造阶段不会发起网络请求。
    ///
    /// # Errors
    ///
    /// issuer、客户端标识或超时不满足边界时返回配置错误。
    pub fn new(config: OidcDeviceGrantConfig) -> Result<Self, OidcDeviceGrantConfigurationError> {
        if config.client_id.is_empty()
            || config.client_id.len() > 512
            || config.client_id.chars().any(char::is_control)
        {
            return Err(OidcDeviceGrantConfigurationError::InvalidClient);
        }
        if config.request_timeout.is_zero()
            || !(MINIMUM_POLLING_DURATION..=MAXIMUM_POLLING_DURATION)
                .contains(&config.maximum_polling_duration)
        {
            return Err(OidcDeviceGrantConfigurationError::InvalidTimeout);
        }
        let issuer = IssuerUrl::new(config.issuer_url)
            .map_err(|_| OidcDeviceGrantConfigurationError::InvalidIssuer)?;
        let http_client = reqwest::ClientBuilder::new()
            .timeout(config.request_timeout)
            .connect_timeout(config.request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| OidcDeviceGrantConfigurationError::HttpClient)?;

        Ok(Self {
            issuer,
            client_id: ClientId::new(config.client_id),
            request_timeout: config.request_timeout,
            maximum_polling_duration: config.maximum_polling_duration,
            http_client,
            metadata: OnceCell::new(),
        })
    }

    async fn provider_metadata(&self) -> OidcResult<&DeviceProviderMetadata> {
        self.metadata
            .get_or_try_init(|| async {
                DeviceProviderMetadata::discover_async(self.issuer.clone(), &self.http_client)
                    .await
                    .map_err(|error| map_discovery_error(&error))
            })
            .await
    }

    async fn authorize_internal(
        &self,
        prompt_sink: &dyn OidcDeviceAuthorizationPromptSink,
    ) -> OidcResult<SecretValue> {
        let metadata = self.provider_metadata().await?;
        let client =
            CoreClient::from_provider_metadata(metadata.clone(), self.client_id.clone(), None)
                .set_device_authorization_url(
                    metadata
                        .additional_metadata()
                        .device_authorization_endpoint
                        .clone(),
                )
                .set_auth_type(AuthType::RequestBody);
        let authorization: CoreDeviceAuthorizationResponse = client
            .exchange_device_code()
            .add_scope(Scope::new("openid".to_owned()))
            .add_scope(Scope::new("profile".to_owned()))
            .request_async(&self.http_client)
            .await
            .map_err(|error| map_token_request_error(&error))?;
        let prompt = prompt(&authorization)?;
        prompt_sink
            .present(&prompt)
            .map_err(|_| OidcFailure::new(OidcFailureKind::ProviderRejected))?;

        let token = client
            .exchange_device_access_token(&authorization)
            .map_err(|_| OidcFailure::new(OidcFailureKind::InvalidConfiguration))?
            .set_max_backoff_interval(self.request_timeout)
            .request_async(
                &self.http_client,
                tokio::time::sleep,
                Some(
                    self.maximum_polling_duration
                        .min(authorization.expires_in()),
                ),
            )
            .await
            .map_err(|error| map_token_request_error(&error))?;
        let id_token = token
            .extra_fields()
            .id_token()
            .ok_or_else(invalid_identity_token)?;
        SecretValue::new(id_token.to_string()).map_err(|_| invalid_identity_token())
    }

    async fn verify_assertion_internal(
        &self,
        assertion: &SecretValue,
    ) -> OidcResult<VerifiedOidcIdentity> {
        let metadata = self.provider_metadata().await?;
        let client =
            CoreClient::from_provider_metadata(metadata.clone(), self.client_id.clone(), None);
        let id_token =
            CoreIdToken::from_str(assertion.expose()).map_err(|_| invalid_identity_token())?;
        let verifier = client.id_token_verifier();
        let claims = id_token
            .claims(&verifier, |nonce: Option<&Nonce>| {
                if nonce.is_none() {
                    Ok(())
                } else {
                    Err("设备授权断言包含未请求的 nonce".to_owned())
                }
            })
            .map_err(|_| invalid_identity_token())?;
        let authenticated_at = claims
            .auth_time()
            .map(|value| UtcMillis::new(value.timestamp_millis()))
            .transpose()
            .map_err(|_| invalid_identity_token())?;
        let display_name = claims
            .name()
            .and_then(|claim| claim.get(None))
            .map(|name| name.as_str().to_owned())
            .or_else(|| {
                claims
                    .preferred_username()
                    .map(|name| name.as_str().to_owned())
            });
        let locale = claims.locale().map(|locale| locale.as_str().to_owned());

        VerifiedOidcIdentity::new(
            self.issuer.as_str(),
            claims.subject().as_str(),
            display_name,
            locale,
            authenticated_at,
        )
        .map_err(|_| invalid_identity_token())
    }
}

impl OidcDeviceGrantGateway for DiscoveredOidcDeviceGrant {
    fn authorize<'a>(
        &'a self,
        prompt_sink: &'a dyn OidcDeviceAuthorizationPromptSink,
    ) -> PortFuture<'a, OidcResult<SecretValue>> {
        Box::pin(self.authorize_internal(prompt_sink))
    }
}

impl OidcDeviceAssertionVerifier for DiscoveredOidcDeviceGrant {
    fn verify_assertion<'a>(
        &'a self,
        assertion: &'a SecretValue,
    ) -> PortFuture<'a, OidcResult<VerifiedOidcIdentity>> {
        Box::pin(self.verify_assertion_internal(assertion))
    }
}

fn prompt(
    authorization: &CoreDeviceAuthorizationResponse,
) -> OidcResult<OidcDeviceAuthorizationPrompt> {
    Ok(OidcDeviceAuthorizationPrompt {
        user_code: SecretValue::new(authorization.user_code().secret().to_owned())
            .map_err(|_| invalid_identity_token())?,
        verification_uri: authorization.verification_uri().to_string(),
        verification_uri_complete: authorization
            .verification_uri_complete()
            .map(|uri| uri.secret().to_owned()),
        expires_in: duration(authorization.expires_in())?,
        polling_interval: duration(authorization.interval())?,
    })
}

fn duration(value: Duration) -> OidcResult<DurationMillis> {
    let milliseconds = u64::try_from(value.as_millis()).map_err(|_| invalid_identity_token())?;
    DurationMillis::new(milliseconds).map_err(|_| invalid_identity_token())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OidcDeviceGrantConfigurationError {
    #[error("OIDC 设备授权 issuer 配置无效")]
    InvalidIssuer,
    #[error("OIDC 设备授权客户端配置无效")]
    InvalidClient,
    #[error("OIDC 设备授权超时配置无效")]
    InvalidTimeout,
    #[error("OIDC 设备授权 HTTP 客户端初始化失败")]
    HttpClient,
}
