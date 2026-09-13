//! Private, single-owner Streamable HTTP transport. Bridge remains the authority
//! for task identity and tool permissions; transport sessions grant no identity.

use std::{fs::File, io::Read as _, net::SocketAddr, path::Path, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse as _, Response},
};
use futures_util::{StreamExt as _, stream};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use url::{Host, Position, Url};
use zeroize::Zeroizing;

use crate::agent_room::{AgentRoomMcpServer, BridgeToolClient};
use crate::oauth::{OAuth, Rejection};

enum Authentication {
    Token([u8; 32]),
    OAuth(Box<OAuth>),
}

pub struct HttpConfig {
    host: String,
    origin: String,
    authentication: Authentication,
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
        let url = public_endpoint(bind, public_url)?;
        let token = read_token(token_file)?;
        Ok(Self {
            host: url[Position::BeforeHost..Position::AfterPort].to_owned(),
            origin: url.origin().ascii_serialization(),
            authentication: Authentication::Token(Sha256::digest(token.as_bytes()).into()),
        })
    }

    /// Load an explicitly configured, single-owner OAuth resource server.
    /// # Errors
    /// Invalid configuration, unavailable discovery or unsafe endpoints fail startup.
    pub async fn oauth(
        bind: SocketAddr,
        public_url: &str,
        config_file: &Path,
    ) -> Result<Self, &'static str> {
        let url = public_endpoint(bind, public_url)?;
        let authentication = OAuth::load(
            config_file,
            &url,
            bind.ip().is_loopback() && url.scheme() == "http",
        )
        .await?;
        Ok(Self {
            host: url[Position::BeforeHost..Position::AfterPort].to_owned(),
            origin: url.origin().ascii_serialization(),
            authentication: Authentication::OAuth(Box::new(authentication)),
        })
    }
}

fn public_endpoint(bind: SocketAddr, public_url: &str) -> Result<Url, &'static str> {
    let url = Url::parse(public_url).map_err(|_| "public URL is invalid")?;
    let loopback = match url.host() {
        Some(Host::Domain("localhost")) => true,
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    };
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback && bind.ip().is_loopback()))
        || url.path() != "/mcp"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host().is_none()
    {
        return Err("public URL must be HTTPS with path /mcp (HTTP is allowed only on loopback)");
    }
    Ok(url)
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
    permits: Arc<Semaphore>,
}

/// Builds an authenticated router. No Bridge administration or native IPC endpoint
/// is exposed. The caller must terminate TLS at its trusted reverse proxy.
pub fn router(
    backend: Arc<dyn BridgeToolClient>,
    config: HttpConfig,
    shutdown: CancellationToken,
) -> Router {
    let transport_config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_cancellation_token(shutdown)
        // Stream protocol progress and keep idle waits alive with SSE comments.
        .with_json_response(false)
        .with_sse_keep_alive(Some(Duration::from_secs(15)))
        .with_allowed_hosts([config.host.clone()])
        .with_allowed_origins([config.origin.clone()])
        .with_max_request_body_bytes(65_536);
    let service = StreamableHttpService::new(
        move || Ok(AgentRoomMcpServer::new(backend.clone())),
        Arc::new(NeverSessionManager::default()),
        transport_config,
    );
    let mut app = Router::new().route_service("/mcp", service);
    if let Authentication::OAuth(oauth) = &config.authentication {
        let metadata = oauth.metadata();
        app = app.route(
            "/.well-known/oauth-protected-resource/mcp",
            axum::routing::get(move || async { Json(metadata) }),
        );
    }
    app.layer(middleware::from_fn_with_state(
        Arc::new(HttpBoundary {
            config,
            permits: Arc::new(Semaphore::new(32)),
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
    if single_header(headers, header::HOST) != Some(boundary.config.host.as_str())
        || (headers.contains_key(header::ORIGIN)
            && single_header(headers, header::ORIGIN) != Some(boundary.config.origin.as_str()))
    {
        return (StatusCode::FORBIDDEN, "Unexpected origin or host").into_response();
    }
    let Ok(permit) = boundary.permits.clone().try_acquire_owned() else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "1")],
            "Busy",
        )
            .into_response();
    };
    let metadata = request.uri().path() == "/.well-known/oauth-protected-resource/mcp"
        && matches!(*request.method(), Method::GET | Method::HEAD);
    if !metadata {
        let authorization = match &boundary.config.authentication {
            Authentication::Token(expected) => {
                if authorized(headers, expected) {
                    Ok(())
                } else {
                    Err(Rejection::Invalid)
                }
            }
            Authentication::OAuth(oauth) => match bearer(headers) {
                Some(token) => oauth.verify(token).await,
                None => Err(Rejection::Invalid),
            },
        };
        if let Err(error) = authorization {
            return rejected(&boundary.config, error);
        }
    }
    // Bound input collection, not the lifetime of a validated wait tool. Newer MCP versions
    // may defer response headers until their first protocol message even in SSE mode.
    let (parts, body) = request.into_parts();
    let body = match tokio::time::timeout(Duration::from_secs(15), to_bytes(body, 65_536)).await {
        Ok(Ok(body)) => body,
        Ok(Err(_)) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                "Invalid or oversized request body",
            )
                .into_response();
        }
        Err(_) => return (StatusCode::REQUEST_TIMEOUT, "Request body timed out").into_response(),
    };
    let mut response = next.run(Request::from_parts(parts, Body::from(body))).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // The limit covers the entire stream, including idle waits. Dropping this body also drops
    // rmcp's stream, which cancels the handler and its Bridge polling future.
    let (parts, body) = response.into_parts();
    let stream = stream::unfold(
        (body.into_data_stream(), permit),
        |(mut body, permit)| async { body.next().await.map(|chunk| (chunk, (body, permit))) },
    );
    Response::from_parts(parts, Body::from_stream(stream))
}

fn rejected(config: &HttpConfig, rejection: Rejection) -> Response {
    if rejection == Rejection::Unavailable {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [
                (header::RETRY_AFTER, "30"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            "Authentication provider unavailable",
        )
            .into_response();
    }
    let (status, code) = if rejection == Rejection::Scope {
        (StatusCode::FORBIDDEN, "insufficient_scope")
    } else {
        (StatusCode::UNAUTHORIZED, "invalid_token")
    };
    let challenge = match &config.authentication {
        Authentication::Token(_) => "Bearer realm=\"agent-room\"".into(),
        Authentication::OAuth(oauth) => {
            format!("{}, error=\"{code}\"", oauth.challenge(&config.origin))
        }
    };
    (
        status,
        [
            (header::WWW_AUTHENTICATE, challenge),
            (header::CACHE_CONTROL, "no-store".into()),
        ],
        "Authentication required",
    )
        .into_response()
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
    let Some(token) = bearer(headers) else {
        return false;
    };
    if !(43..=128).contains(&token.len()) {
        return false;
    }
    let actual: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    bool::from(actual.ct_eq(expected))
}
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let (scheme, token) = single_header(headers, header::AUTHORIZATION)?.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("Bearer")
        && !token.is_empty()
        && !token.bytes().any(|b| b.is_ascii_whitespace()))
    .then_some(token)
}
