//! Private, single-owner Streamable HTTP transport. Bridge remains the authority
//! for task identity and tool permissions; transport sessions grant no identity.

use std::{fs::File, io::Read as _, net::SocketAddr, path::Path, sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse as _, Response},
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use tokio::sync::Semaphore;
use url::{Host, Position, Url};
use zeroize::Zeroizing;

use crate::agent_room::{AgentRoomMcpServer, BridgeToolClient};

pub struct HttpConfig {
    host: String,
    origin: String,
    token_hash: [u8; 32],
}

impl HttpConfig {
    /// Validates the advertised origin and reads a separately mounted bearer token.
    ///
    /// # Errors
    /// Rejects unsafe origins, weak/missing tokens and non-private Unix token files.
    pub fn load(
        bind: SocketAddr,
        public_url: &str,
        token_file: &Path,
    ) -> Result<Self, &'static str> {
        let url = Url::parse(public_url).map_err(|_| "public URL is invalid")?;
        let loopback = match url.host() {
            Some(Host::Domain("localhost")) => true,
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            _ => false,
        };
        if (url.scheme() != "https"
            && !(url.scheme() == "http" && loopback && bind.ip().is_loopback()))
            || url.path() != "/mcp"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.host().is_none()
        {
            return Err(
                "public URL must be HTTPS with path /mcp (HTTP is allowed only on loopback)",
            );
        }
        let token = read_token(token_file)?;
        Ok(Self {
            host: url[Position::BeforeHost..Position::AfterPort].to_owned(),
            origin: url.origin().ascii_serialization(),
            token_hash: Sha256::digest(token.as_bytes()).into(),
        })
    }
}

fn read_token(path: &Path) -> Result<Zeroizing<String>, &'static str> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "cannot read token file")?;
    if !path.is_absolute() || !metadata.is_file() || metadata.len() > 256 {
        return Err("token file must be a regular, absolute file of at most 256 bytes");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("token file must be owner-only");
        }
    }
    let mut contents = Zeroizing::new(String::new());
    File::open(path)
        .and_then(|file| file.take(257).read_to_string(&mut contents))
        .map_err(|_| "cannot read token file")?;
    let value = contents.trim_end_matches(['\r', '\n']);
    if !(43..=128).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("token must contain 43 to 128 random URL-safe ASCII characters");
    }
    Ok(Zeroizing::new(value.to_owned()))
}

struct HttpBoundary {
    config: HttpConfig,
    permits: Semaphore,
}

/// Builds an authenticated router. No Bridge administration or native IPC endpoint
/// is exposed. The caller must terminate TLS at its trusted reverse proxy.
pub fn router(backend: Arc<dyn BridgeToolClient>, config: HttpConfig) -> Router {
    let transport_config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_allowed_hosts([config.host.clone()])
        .with_allowed_origins([config.origin.clone()])
        .with_max_request_body_bytes(65_536);
    let service = StreamableHttpService::new(
        move || Ok(AgentRoomMcpServer::new(backend.clone())),
        Arc::new(NeverSessionManager::default()),
        transport_config,
    );
    Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn_with_state(
            Arc::new(HttpBoundary {
                config,
                permits: Semaphore::new(32),
            }),
            protect,
        ))
}

async fn protect(
    State(boundary): State<Arc<HttpBoundary>>,
    request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    if !authorized(headers, &boundary.config.token_hash) {
        return (
            StatusCode::UNAUTHORIZED,
            [
                (header::WWW_AUTHENTICATE, "Bearer realm=\"agent-room\""),
                (header::CACHE_CONTROL, "no-store"),
            ],
            "Authentication required",
        )
            .into_response();
    }
    if single_header(headers, header::HOST) != Some(boundary.config.host.as_str())
        || (headers.contains_key(header::ORIGIN)
            && single_header(headers, header::ORIGIN) != Some(boundary.config.origin.as_str()))
    {
        return (StatusCode::FORBIDDEN, "Unexpected origin or host").into_response();
    }
    let Ok(_permit) = boundary.permits.try_acquire() else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "1")],
            "Busy",
        )
            .into_response();
    };
    let mut response = tokio::time::timeout(Duration::from_secs(155), next.run(request))
        .await
        .unwrap_or_else(|_| (StatusCode::GATEWAY_TIMEOUT, "Request timed out").into_response());
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn single_header(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}

fn authorized(headers: &HeaderMap, expected: &[u8; 32]) -> bool {
    let Some(value) = single_header(headers, header::AUTHORIZATION) else {
        return false;
    };
    let Some((scheme, token)) = value.split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("Bearer") || !(43..=128).contains(&token.len()) {
        return false;
    }
    let actual: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    bool::from(actual.ct_eq(expected))
}
