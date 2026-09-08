//! A single-owner OAuth resource server. Tokens never become Bridge credentials.
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    jwk::{Jwk, JwkSet, KeyOperations, PublicKeyUse},
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
use tokio::{sync::Mutex, time::Instant};
use url::Url;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Settings {
    issuer: String,
    owner_subject: String,
    allowed_client_ids: Vec<String>,
    scope: String,
}
#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: String,
    authorization_endpoint: String,
    token_endpoint: String,
    code_challenge_methods_supported: Vec<String>,
}
struct Keys {
    set: JwkSet,
    fetched: Instant,
    attempted: Instant,
}
pub(crate) struct OAuth {
    settings: Settings,
    resource: String,
    client: reqwest::Client,
    jwks_url: Url,
    keys: Mutex<Keys>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rejection {
    Invalid,
    Scope,
    Unavailable,
}
#[derive(Deserialize)]
struct Claims {
    scope: Option<String>,
    azp: Option<String>,
    client_id: Option<String>,
    typ: Option<String>,
}

impl OAuth {
    pub(crate) async fn load(
        path: &Path,
        resource: &Url,
        local_http: bool,
    ) -> Result<Self, &'static str> {
        let metadata =
            std::fs::symlink_metadata(path).map_err(|_| "cannot read OAuth configuration")?;
        if !path.is_absolute() || !metadata.is_file() || metadata.len() > 65_536 {
            return Err("OAuth configuration must be a regular absolute file of at most 64 KiB");
        }
        let settings: Settings = serde_json::from_slice(
            &std::fs::read(path).map_err(|_| "cannot read OAuth configuration")?,
        )
        .map_err(|_| "OAuth configuration is invalid")?;
        if settings.owner_subject.is_empty()
            || settings.owner_subject.len() > 255
            || settings.allowed_client_ids.is_empty()
            || settings.allowed_client_ids.len() > 32
            || settings.allowed_client_ids.iter().any(|value| {
                value.is_empty() || value.len() > 255 || value.chars().any(char::is_control)
            })
            || settings.scope.is_empty()
            || settings.scope.len() > 128
            || !settings
                .scope
                .bytes()
                .all(|byte| matches!(byte, 0x21 | 0x23..=0x5b | 0x5d..=0x7e))
        {
            return Err("OAuth owner, clients and scope must be explicit and valid");
        }
        let issuer = endpoint(&settings.issuer, local_http)?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|_| "cannot initialize OAuth HTTP client")?;
        let discovery_url = Url::parse(&format!(
            "{}/.well-known/openid-configuration",
            settings.issuer.trim_end_matches('/')
        ))
        .map_err(|_| "OAuth discovery URL is invalid")?;
        let discovery: Discovery = fetch(&client, &discovery_url).await?;
        if discovery.issuer != settings.issuer
            || !discovery
                .code_challenge_methods_supported
                .iter()
                .any(|method| method == "S256")
        {
            return Err("OAuth discovery must match issuer and support PKCE S256");
        }
        let jwks_url = endpoint(&discovery.jwks_uri, local_http)?;
        for target in [
            &jwks_url,
            &endpoint(&discovery.authorization_endpoint, local_http)?,
            &endpoint(&discovery.token_endpoint, local_http)?,
        ] {
            if target.origin() != issuer.origin() {
                return Err("OAuth discovery endpoints must share the configured issuer origin");
            }
        }
        let set = fetch_keys(&client, &jwks_url).await?;
        Ok(Self {
            settings,
            resource: resource.as_str().into(),
            client,
            jwks_url,
            keys: Mutex::new(Keys {
                set,
                fetched: Instant::now(),
                attempted: Instant::now(),
            }),
        })
    }

    pub(crate) fn metadata(&self) -> Value {
        json!({"resource": self.resource, "authorization_servers": [self.settings.issuer],
            "scopes_supported": [self.settings.scope], "bearer_methods_supported": ["header"],
            "resource_name": "Agent Room"})
    }
    pub(crate) fn challenge(&self, origin: &str) -> String {
        format!(
            "Bearer resource_metadata=\"{origin}/.well-known/oauth-protected-resource/mcp\", scope=\"{}\"",
            self.settings.scope
        )
    }
    pub(crate) async fn verify(&self, token: &str) -> Result<(), Rejection> {
        if token.len() > 16_384 {
            return Err(Rejection::Invalid);
        }
        let header = decode_header(token).map_err(|_| Rejection::Invalid)?;
        if header.alg != Algorithm::RS256 {
            return Err(Rejection::Invalid);
        }
        let kid = header
            .kid
            .as_deref()
            .filter(|kid| !kid.is_empty() && kid.len() <= 256)
            .ok_or(Rejection::Invalid)?;
        let key = self.key(kid).await?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = 30;
        validation.validate_nbf = true;
        validation.set_required_spec_claims(&["exp", "iss", "sub", "aud"]);
        validation.set_issuer(&[&self.settings.issuer]);
        validation.set_audience(&[&self.resource]);
        validation.sub = Some(self.settings.owner_subject.clone());
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|_| Rejection::Invalid)?
            .claims;
        // Keycloak marks access tokens as Bearer; RFC 9068 uses an at+jwt header.
        // ID tokens and tokens issued to a different host must never authorize tools.
        if header.typ.as_deref() != Some("at+jwt") && claims.typ.as_deref() != Some("Bearer") {
            return Err(Rejection::Invalid);
        }
        let client_id = match (&claims.azp, &claims.client_id) {
            (Some(a), Some(b)) if a != b => return Err(Rejection::Invalid),
            (Some(value), _) | (_, Some(value)) => value,
            _ => return Err(Rejection::Invalid),
        };
        if !self.settings.allowed_client_ids.contains(client_id) {
            return Err(Rejection::Invalid);
        }
        if !claims.scope.as_deref().is_some_and(|scope| {
            scope
                .split_ascii_whitespace()
                .any(|s| s == self.settings.scope)
        }) {
            return Err(Rejection::Scope);
        }
        Ok(())
    }
    async fn key(&self, kid: &str) -> Result<DecodingKey, Rejection> {
        let mut keys = self.keys.lock().await;
        let stale = keys.fetched.elapsed() >= Duration::from_mins(5);
        let unknown = !keys
            .set
            .keys
            .iter()
            .any(|key| key.common.key_id.as_deref() == Some(kid));
        if stale || unknown {
            if keys.attempted.elapsed() >= Duration::from_secs(30) {
                keys.attempted = Instant::now();
                keys.set = fetch_keys(&self.client, &self.jwks_url)
                    .await
                    .map_err(|_| Rejection::Unavailable)?;
                keys.fetched = Instant::now();
            } else if stale {
                return Err(Rejection::Unavailable);
            }
        }
        let mut matching = keys
            .set
            .keys
            .iter()
            .filter(|key| key.common.key_id.as_deref() == Some(kid));
        let key = matching.next().ok_or(Rejection::Invalid)?;
        if matching.next().is_some() || !signing_key(key) {
            return Err(Rejection::Invalid);
        }
        DecodingKey::from_jwk(key).map_err(|_| Rejection::Invalid)
    }
}
fn signing_key(key: &Jwk) -> bool {
    key.common
        .public_key_use
        .as_ref()
        .is_none_or(|usage| *usage == PublicKeyUse::Signature)
        && key
            .common
            .key_operations
            .as_ref()
            .is_none_or(|ops| ops.as_slice() == [KeyOperations::Verify])
        && key
            .common
            .key_algorithm
            .is_none_or(|alg| Algorithm::try_from(alg).ok() == Some(Algorithm::RS256))
}
fn endpoint(value: &str, local_http: bool) -> Result<Url, &'static str> {
    let url = Url::parse(value).map_err(|_| "OAuth endpoint is invalid")?;
    let loopback = match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (url.scheme() != "https" && !(local_http && loopback && url.scheme() == "http"))
    {
        return Err("OAuth endpoints require HTTPS (HTTP is restricted to local development)");
    }
    Ok(url)
}
async fn fetch_keys(client: &reqwest::Client, url: &Url) -> Result<JwkSet, &'static str> {
    let keys: JwkSet = fetch(client, url).await?;
    if keys.keys.is_empty() || keys.keys.len() > 64 {
        return Err("OAuth key set size is invalid");
    }
    Ok(keys)
}
async fn fetch<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &Url,
) -> Result<T, &'static str> {
    let mut response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|_| "OAuth provider unavailable")?
        .error_for_status()
        .map_err(|_| "OAuth provider rejected discovery")?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "OAuth document incomplete")?
    {
        if bytes.len() + chunk.len() > 262_144 {
            return Err("OAuth document exceeds 256 KiB");
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "OAuth provider document invalid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::get};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn public_keys(kid: &str) -> JwkSet {
        // Key refresh tests use public JWK-shaped data; actual RS256 signatures are tested over HTTP.
        serde_json::from_value(json!({"keys":[{"kty":"RSA","kid":kid,"n":"AQAB","e":"AQAB","use":"sig","alg":"RS256"}]})).unwrap()
    }
    #[tokio::test]
    async fn key_rotation_is_throttled_and_stale_keys_fail_closed() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let app = Router::new().route(
            "/keys",
            get(move || {
                counter.fetch_add(1, Ordering::Relaxed);
                async { Json(public_keys("rotated")) }
            }),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let oauth = OAuth {
            settings: Settings {
                issuer: url.clone(),
                owner_subject: "owner".into(),
                allowed_client_ids: vec!["client".into()],
                scope: "agent-room".into(),
            },
            resource: "https://agents.example/mcp".into(),
            client: reqwest::Client::new(),
            jwks_url: Url::parse(&format!("{url}/keys")).unwrap(),
            keys: Mutex::new(Keys {
                set: public_keys("initial"),
                fetched: Instant::now(),
                attempted: Instant::now(),
            }),
        };
        assert!(oauth.key("initial").await.is_ok());
        for _ in 0..4 {
            assert!(matches!(
                oauth.key("rotated").await,
                Err(Rejection::Invalid)
            ));
        }
        assert_eq!(requests.load(Ordering::Relaxed), 0);
        oauth.keys.lock().await.attempted = Instant::now() - Duration::from_secs(31);
        assert!(oauth.key("rotated").await.is_ok());
        assert_eq!(requests.load(Ordering::Relaxed), 1);
        assert!(matches!(
            oauth.key("initial").await,
            Err(Rejection::Invalid)
        ));
        server.abort();
        let _ = server.await;
        {
            let mut keys = oauth.keys.lock().await;
            keys.fetched = Instant::now() - Duration::from_mins(6);
            keys.attempted = Instant::now() - Duration::from_secs(31);
        }
        assert!(matches!(
            oauth.key("rotated").await,
            Err(Rejection::Unavailable)
        ));
        assert!(matches!(
            oauth.key("rotated").await,
            Err(Rejection::Unavailable)
        ));
    }
}
