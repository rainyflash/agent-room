use std::{error::Error, time::Duration};

use agent_room_application::ports::{
    OidcAuthorizationOptions, OidcAuthorizationRequest, OidcCodeExchange, OidcFailure,
    OidcFailureKind, OidcGateway, OidcResult, PortFuture, SecretDigest, SecretFactory,
    SecretGenerationFailure, SecretValue, VerifiedOidcIdentity,
};
use agent_room_domain::time::UtcMillis;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use openidconnect::{
    AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, DiscoveryError,
    IssuerUrl, JsonWebKey, Nonce, OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, RequestTokenError, Scope, TokenResponse,
    core::{
        CoreAuthenticationFlow, CoreClient, CoreJsonWebKey, CoreJwsSigningAlgorithm,
        CoreProviderMetadata,
    },
    reqwest,
};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::OnceCell;

const SECRET_ENTROPY_BYTES: usize = 32;

pub struct SecureSecretFactory;

impl SecretFactory for SecureSecretFactory {
    fn generate(&self) -> Result<SecretValue, SecretGenerationFailure> {
        let mut bytes = [0_u8; SECRET_ENTROPY_BYTES];
        getrandom::fill(&mut bytes).map_err(|_| SecretGenerationFailure::EntropyUnavailable)?;
        SecretValue::new(URL_SAFE_NO_PAD.encode(bytes))
            .map_err(|_| SecretGenerationFailure::EntropyUnavailable)
    }

    fn digest(&self, value: &str) -> SecretDigest {
        SecretDigest::from_array(Sha256::digest(value.as_bytes()).into())
    }
}

pub struct OidcAdapterConfig {
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: SecretValue,
    pub redirect_url: String,
    pub request_timeout: Duration,
}

pub struct DiscoveredOidcGateway {
    issuer: IssuerUrl,
    client_id: ClientId,
    client_secret: ClientSecret,
    redirect_url: RedirectUrl,
    http_client: reqwest::Client,
    metadata: OnceCell<CoreProviderMetadata>,
}

impl DiscoveredOidcGateway {
    /// 创建惰性 OIDC Discovery 适配器。构造阶段只验证本地配置，不发网络请求。
    ///
    /// # Errors
    ///
    /// URL、客户端标识、超时或 HTTP 客户端配置无效时返回稳定配置错误。
    pub fn new(config: OidcAdapterConfig) -> Result<Self, OidcAdapterConfigurationError> {
        if config.client_id.is_empty()
            || config.client_id.len() > 512
            || config.client_id.chars().any(char::is_control)
            || config.request_timeout.is_zero()
        {
            return Err(OidcAdapterConfigurationError::InvalidClient);
        }
        let issuer = IssuerUrl::new(config.issuer_url)
            .map_err(|_| OidcAdapterConfigurationError::InvalidIssuer)?;
        let redirect_url = RedirectUrl::new(config.redirect_url)
            .map_err(|_| OidcAdapterConfigurationError::InvalidRedirectUrl)?;
        let http_client = reqwest::ClientBuilder::new()
            .timeout(config.request_timeout)
            .connect_timeout(config.request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| OidcAdapterConfigurationError::HttpClient)?;

        Ok(Self {
            issuer,
            client_id: ClientId::new(config.client_id),
            client_secret: ClientSecret::new(config.client_secret.expose().to_owned()),
            redirect_url,
            http_client,
            metadata: OnceCell::new(),
        })
    }

    async fn provider_metadata(&self) -> OidcResult<&CoreProviderMetadata> {
        self.metadata
            .get_or_try_init(|| async {
                CoreProviderMetadata::discover_async(self.issuer.clone(), &self.http_client)
                    .await
                    .map_err(|error| map_discovery_error(&error))
            })
            .await
    }
}

impl OidcGateway for DiscoveredOidcGateway {
    fn begin_authorization(
        &self,
        options: OidcAuthorizationOptions,
    ) -> PortFuture<'_, OidcResult<OidcAuthorizationRequest>> {
        Box::pin(async move {
            let metadata = self.provider_metadata().await?;
            let client = CoreClient::from_provider_metadata(
                metadata.clone(),
                self.client_id.clone(),
                Some(self.client_secret.clone()),
            )
            .set_redirect_uri(self.redirect_url.clone());
            let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
            let authorization = client
                .authorize_url(
                    CoreAuthenticationFlow::AuthorizationCode,
                    CsrfToken::new_random,
                    Nonce::new_random,
                )
                .set_max_age(Duration::from_millis(
                    options.maximum_authentication_age.value(),
                ))
                .set_pkce_challenge(pkce_challenge);
            let authorization = if options.request_profile {
                authorization.add_scope(Scope::new("profile".to_owned()))
            } else {
                authorization
            };
            let (authorization_url, state, nonce) = authorization.url();

            Ok(OidcAuthorizationRequest {
                authorization_url: authorization_url.to_string(),
                state: secret_value(state.secret())?,
                nonce: secret_value(nonce.secret())?,
                pkce_verifier: secret_value(pkce_verifier.secret())?,
            })
        })
    }

    fn exchange_code<'a>(
        &'a self,
        exchange: OidcCodeExchange<'a>,
    ) -> PortFuture<'a, OidcResult<VerifiedOidcIdentity>> {
        Box::pin(async move {
            let metadata = self.provider_metadata().await?;
            let client = CoreClient::from_provider_metadata(
                metadata.clone(),
                self.client_id.clone(),
                Some(self.client_secret.clone()),
            )
            .set_redirect_uri(self.redirect_url.clone());
            let token_response = client
                .exchange_code(AuthorizationCode::new(exchange.code.to_owned()))
                .map_err(|_| OidcFailure::new(OidcFailureKind::InvalidConfiguration))?
                .set_pkce_verifier(PkceCodeVerifier::new(
                    exchange.pkce_verifier.expose().to_owned(),
                ))
                .request_async(&self.http_client)
                .await
                .map_err(|error| map_token_request_error(&error))?;
            let id_token = token_response
                .id_token()
                .ok_or_else(|| OidcFailure::new(OidcFailureKind::InvalidIdentityToken))?;
            let verifier = client.id_token_verifier();
            let nonce = Nonce::new(exchange.expected_nonce.expose().to_owned());
            let claims = id_token
                .claims(&verifier, &nonce)
                .map_err(|_| OidcFailure::new(OidcFailureKind::InvalidIdentityToken))?;

            if let Some(expected_hash) = claims.access_token_hash() {
                let signing_algorithm = id_token
                    .signing_alg()
                    .map_err(|_| invalid_identity_token())?;
                let actual_hash = if is_hmac(signing_algorithm) {
                    let symmetric_key = CoreJsonWebKey::new_symmetric(
                        self.client_secret.secret().as_bytes().to_vec(),
                    );
                    AccessTokenHash::from_token(
                        token_response.access_token(),
                        signing_algorithm,
                        &symmetric_key,
                    )
                } else {
                    let signing_key = id_token
                        .signing_key(&verifier)
                        .map_err(|_| invalid_identity_token())?;
                    AccessTokenHash::from_token(
                        token_response.access_token(),
                        signing_algorithm,
                        signing_key,
                    )
                }
                .map_err(|_| invalid_identity_token())?;
                if actual_hash != *expected_hash {
                    return Err(invalid_identity_token());
                }
            }

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
        })
    }
}

fn secret_value(value: &str) -> OidcResult<SecretValue> {
    SecretValue::new(value.to_owned())
        .map_err(|_| OidcFailure::new(OidcFailureKind::InvalidConfiguration))
}

fn map_discovery_error<E>(error: &DiscoveryError<E>) -> OidcFailure
where
    E: Error + 'static,
{
    let kind = if matches!(error, DiscoveryError::Request(_)) {
        OidcFailureKind::DependencyUnavailable
    } else {
        OidcFailureKind::InvalidConfiguration
    };
    OidcFailure::new(kind)
}

fn map_token_request_error<E, T>(error: &RequestTokenError<E, T>) -> OidcFailure
where
    E: Error + 'static,
    T: openidconnect::ErrorResponse + 'static,
{
    let kind = match error {
        RequestTokenError::Request(_) => OidcFailureKind::DependencyUnavailable,
        RequestTokenError::ServerResponse(_) => OidcFailureKind::ProviderRejected,
        RequestTokenError::Parse(_, _) | RequestTokenError::Other(_) => {
            OidcFailureKind::InvalidIdentityToken
        }
    };
    OidcFailure::new(kind)
}

const fn invalid_identity_token() -> OidcFailure {
    OidcFailure::new(OidcFailureKind::InvalidIdentityToken)
}

const fn is_hmac(algorithm: &CoreJwsSigningAlgorithm) -> bool {
    matches!(
        algorithm,
        CoreJwsSigningAlgorithm::HmacSha256
            | CoreJwsSigningAlgorithm::HmacSha384
            | CoreJwsSigningAlgorithm::HmacSha512
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum OidcAdapterConfigurationError {
    #[error("OIDC issuer 配置无效")]
    InvalidIssuer,
    #[error("OIDC 客户端配置无效")]
    InvalidClient,
    #[error("OIDC 回调地址配置无效")]
    InvalidRedirectUrl,
    #[error("OIDC HTTP 客户端初始化失败")]
    HttpClient,
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{SecretFactory, SecretValue};

    use super::{OidcAdapterConfig, SecureSecretFactory};

    #[test]
    fn 会话密钥具有足够熵且调试输出脱敏() {
        let factory = SecureSecretFactory;
        let first = factory.generate().expect("系统随机源可用");
        let second = factory.generate().expect("系统随机源可用");

        assert_ne!(first, second);
        assert!(first.expose().len() >= 43);
        assert_eq!(format!("{first:?}"), "[已脱敏]");
        assert_ne!(
            factory.digest(first.expose()),
            factory.digest(second.expose())
        );
    }

    #[test]
    fn 构造阶段拒绝非法_oidc_地址() {
        let result = super::DiscoveredOidcGateway::new(OidcAdapterConfig {
            issuer_url: "not-a-url".to_owned(),
            client_id: "agent-room".to_owned(),
            client_secret: SecretValue::new("secret").expect("密钥有效"),
            redirect_url: "https://app.example/callback".to_owned(),
            request_timeout: std::time::Duration::from_secs(2),
        });

        assert!(result.is_err());
    }
}
