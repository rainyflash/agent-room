use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use cookie::{Cookie, SameSite, time::OffsetDateTime};
use keyring::{Entry, Error as KeyringError};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use tauri::{AppHandle, Emitter as _, Manager as _};
use tauri_plugin_opener::OpenerExt as _;
use url::Url;

use crate::{
    desktop_config::DesktopBridgeConfig,
    loopback_callback::{LoopbackCallbackFailure, LoopbackCallbackListener},
};

pub(crate) const HUMAN_SESSION_CHANGED_EVENT: &str = "desktop://human-session-changed";
pub(crate) const HUMAN_SESSION_FAILED_EVENT: &str = "desktop://human-session-failed";

const DESKTOP_SESSION_COOKIE: &str = "__Secure-agent-room-desktop-session";
const PENDING_AUTHENTICATION_ACCOUNT: &str = "human-authentication-pending-v1";
const HUMAN_SESSION_ACCOUNT: &str = "human-session-v1";
const PENDING_AUTHENTICATION_TTL: Duration = Duration::from_mins(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MIN_RANDOM_VALUE_LENGTH: usize = 32;
const MAX_RANDOM_VALUE_LENGTH: usize = 128;
const MAX_AUTHORIZATION_CODE_LENGTH: usize = 4_096;
const MAX_RETURN_PATH_LENGTH: usize = 2_048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DesktopAuthenticationIntent {
    Register,
    SignIn,
}

impl DesktopAuthenticationIntent {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Register => "register",
            Self::SignIn => "sign-in",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PendingAuthentication {
    client_state: String,
    pkce_verifier: String,
    return_path: String,
    created_at_unix_ms: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedHumanSession {
    session_secret: String,
    expires_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HumanAuthenticationCallback {
    authorization_code: String,
    client_state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HumanSessionChanged {
    pub(crate) return_path: String,
    pub(crate) session: HumanSessionView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HumanSessionView {
    authenticated_at_unix_ms: i64,
    display_name: String,
    expires_at_unix_ms: i64,
    locale: String,
    matrix_user_id: String,
    principal_id: String,
    recently_authenticated: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DesktopExchangeResponse {
    session_secret: String,
    session: HumanSessionView,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopExchangeRequest<'a> {
    authorization_code: &'a str,
    pkce_verifier: &'a str,
}

trait HumanSessionVault: Send + Sync {
    fn load_pending(&self) -> HumanSessionResult<Option<PendingAuthentication>>;
    fn write_pending(&self, pending: &PendingAuthentication) -> HumanSessionResult<()>;
    fn delete_pending(&self) -> HumanSessionResult<()>;
    fn load_session(&self) -> HumanSessionResult<Option<PersistedHumanSession>>;
    fn write_session(&self, session: &PersistedHumanSession) -> HumanSessionResult<()>;
    fn delete_session(&self) -> HumanSessionResult<()>;
}

struct KeyringHumanSessionVault {
    service: String,
}

impl KeyringHumanSessionVault {
    fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn read<TValue: DeserializeOwned>(&self, account: &str) -> HumanSessionResult<Option<TValue>> {
        let entry = Entry::new(&self.service, account).map_err(|_| unavailable_vault())?;
        let serialized = match entry.get_password() {
            Ok(value) => value,
            Err(KeyringError::NoEntry) => return Ok(None),
            Err(_) => return Err(unavailable_vault()),
        };
        serde_json::from_str(&serialized)
            .map(Some)
            .map_err(|_| HumanSessionFailure::new("desktop.human_session.vault_corrupt", false))
    }

    fn write<TValue: Serialize>(&self, account: &str, value: &TValue) -> HumanSessionResult<()> {
        let serialized = serde_json::to_string(value).map_err(|_| {
            HumanSessionFailure::new("desktop.human_session.serialize_failed", false)
        })?;
        Entry::new(&self.service, account)
            .and_then(|entry| entry.set_password(&serialized))
            .map_err(|_| unavailable_vault())
    }

    fn delete(&self, account: &str) -> HumanSessionResult<()> {
        let entry = Entry::new(&self.service, account).map_err(|_| unavailable_vault())?;
        match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(_) => Err(unavailable_vault()),
        }
    }
}

impl HumanSessionVault for KeyringHumanSessionVault {
    fn load_pending(&self) -> HumanSessionResult<Option<PendingAuthentication>> {
        self.read(PENDING_AUTHENTICATION_ACCOUNT)
    }

    fn write_pending(&self, pending: &PendingAuthentication) -> HumanSessionResult<()> {
        self.write(PENDING_AUTHENTICATION_ACCOUNT, pending)
    }

    fn delete_pending(&self) -> HumanSessionResult<()> {
        self.delete(PENDING_AUTHENTICATION_ACCOUNT)
    }

    fn load_session(&self) -> HumanSessionResult<Option<PersistedHumanSession>> {
        self.read(HUMAN_SESSION_ACCOUNT)
    }

    fn write_session(&self, session: &PersistedHumanSession) -> HumanSessionResult<()> {
        self.write(HUMAN_SESSION_ACCOUNT, session)
    }

    fn delete_session(&self) -> HumanSessionResult<()> {
        self.delete(HUMAN_SESSION_ACCOUNT)
    }
}

#[derive(Clone)]
pub(crate) struct HumanSessionRuntime {
    control_plane_url: Url,
    browser_control_plane_url: Url,
    http: Client,
    vault: Arc<dyn HumanSessionVault>,
    operation_gate: Arc<Mutex<()>>,
}

impl HumanSessionRuntime {
    pub(crate) fn system(config: &DesktopBridgeConfig) -> HumanSessionResult<Self> {
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| HumanSessionFailure::new("desktop.human_session.http_invalid", false))?;
        Ok(Self {
            control_plane_url: config.control_plane_url(),
            browser_control_plane_url: config.browser_control_plane_url(),
            http,
            vault: Arc::new(KeyringHumanSessionVault::new(
                config.human_session_storage_service(),
            )),
            operation_gate: Arc::new(Mutex::new(())),
        })
    }

    pub(crate) async fn begin_authentication(
        &self,
        app: &AppHandle,
        return_path: &str,
        intent: DesktopAuthenticationIntent,
    ) -> HumanSessionResult<()> {
        validate_return_path(return_path)?;
        let callback_listener = LoopbackCallbackListener::bind().await.map_err(|_| {
            HumanSessionFailure::new("desktop.human_session.loopback_bind_failed", true)
        })?;
        let callback_url = callback_listener.callback_url().to_string();
        let (authorization_url, client_state) = {
            let _guard = self
                .operation_gate
                .lock()
                .map_err(|_| state_unavailable())?;
            let client_state = random_url_safe_value()?;
            let pkce_verifier = random_url_safe_value()?;
            let pending = PendingAuthentication {
                client_state: client_state.clone(),
                pkce_verifier: pkce_verifier.clone(),
                return_path: return_path.to_owned(),
                created_at_unix_ms: now_unix_ms()?,
            };
            self.vault.write_pending(&pending)?;
            let mut authorization_url = self
                .browser_control_plane_url
                .join("auth/desktop/start")
                .map_err(|_| {
                HumanSessionFailure::new("desktop.human_session.url_invalid", false)
            })?;
            authorization_url.query_pairs_mut().extend_pairs([
                ("clientState", client_state.as_str()),
                ("codeChallenge", pkce_challenge(&pkce_verifier).as_str()),
                ("returnTo", return_path),
                ("importDisplayName", "true"),
                ("importLocale", "true"),
                ("intent", intent.as_str()),
                ("callbackUrl", callback_url.as_str()),
            ]);
            (authorization_url, client_state)
        };
        if app
            .opener()
            .open_url(authorization_url.as_str(), None::<&str>)
            .is_err()
        {
            self.cancel_pending_if_current(&client_state)?;
            return Err(HumanSessionFailure::new(
                "desktop.human_session.browser_open_failed",
                true,
            ));
        }
        let handle = app.clone();
        let sessions = self.clone();
        tauri::async_runtime::spawn(async move {
            sessions
                .receive_loopback_callback(handle, callback_listener)
                .await;
        });
        Ok(())
    }

    pub(crate) async fn complete_authentication(
        &self,
        app: &AppHandle,
        callback: HumanAuthenticationCallback,
    ) -> HumanSessionResult<HumanSessionChanged> {
        let pending = {
            let _guard = self
                .operation_gate
                .lock()
                .map_err(|_| state_unavailable())?;
            let pending = self.vault.load_pending()?.ok_or_else(|| {
                HumanSessionFailure::new("desktop.human_session.pending_missing", false)
            })?;
            if let Err(failure) = validate_pending(&pending, &callback, now_unix_ms()?) {
                if failure.code() == "desktop.human_session.pending_expired" {
                    self.vault.delete_pending()?;
                }
                return Err(failure);
            }
            pending
        };
        let exchange = self.exchange(&callback, &pending).await?;
        let persisted = PersistedHumanSession {
            session_secret: exchange.session_secret,
            expires_at_unix_ms: exchange.session.expires_at_unix_ms,
        };
        validate_persisted_session(&persisted, now_unix_ms()?)?;
        {
            let _guard = self
                .operation_gate
                .lock()
                .map_err(|_| state_unavailable())?;
            self.vault.write_session(&persisted)?;
            self.vault.delete_pending()?;
        }
        self.install_cookie(app, &persisted)?;
        Ok(HumanSessionChanged {
            return_path: pending.return_path,
            session: exchange.session,
        })
    }

    pub(crate) fn restore(&self, app: &AppHandle) -> HumanSessionResult<bool> {
        let _guard = self
            .operation_gate
            .lock()
            .map_err(|_| state_unavailable())?;
        let Some(session) = self.vault.load_session()? else {
            self.delete_cookie(app)?;
            return Ok(false);
        };
        if validate_persisted_session(&session, now_unix_ms()?).is_err() {
            self.vault.delete_session()?;
            self.delete_cookie(app)?;
            return Ok(false);
        }
        self.install_cookie(app, &session)?;
        Ok(true)
    }

    pub(crate) fn clear(&self, app: &AppHandle) -> HumanSessionResult<()> {
        let _guard = self
            .operation_gate
            .lock()
            .map_err(|_| state_unavailable())?;
        self.vault.delete_session()?;
        self.vault.delete_pending()?;
        self.delete_cookie(app)
    }

    async fn exchange(
        &self,
        callback: &HumanAuthenticationCallback,
        pending: &PendingAuthentication,
    ) -> HumanSessionResult<DesktopExchangeResponse> {
        let endpoint = self
            .control_plane_url
            .join("/auth/desktop/exchange")
            .map_err(|_| HumanSessionFailure::new("desktop.human_session.url_invalid", false))?;
        let response = self
            .http
            .post(endpoint)
            .json(&DesktopExchangeRequest {
                authorization_code: &callback.authorization_code,
                pkce_verifier: &pending.pkce_verifier,
            })
            .send()
            .await
            .map_err(|_| {
                HumanSessionFailure::new("desktop.human_session.exchange_unavailable", true)
            })?;
        if !response.status().is_success() {
            return Err(exchange_failure(response.status()));
        }
        response
            .json::<DesktopExchangeResponse>()
            .await
            .map_err(|_| HumanSessionFailure::new("desktop.human_session.exchange_invalid", false))
    }

    fn install_cookie(
        &self,
        app: &AppHandle,
        session: &PersistedHumanSession,
    ) -> HumanSessionResult<()> {
        let window = app.get_webview_window("main").ok_or_else(|| {
            HumanSessionFailure::new("desktop.human_session.window_missing", true)
        })?;
        window
            .set_cookie(self.cookie(session)?)
            .map_err(|_| HumanSessionFailure::new("desktop.human_session.cookie_failed", true))
    }

    fn delete_cookie(&self, app: &AppHandle) -> HumanSessionResult<()> {
        let Some(window) = app.get_webview_window("main") else {
            return Ok(());
        };
        window
            .delete_cookie(self.cookie_identity()?)
            .map_err(|_| HumanSessionFailure::new("desktop.human_session.cookie_failed", true))
    }

    fn cookie(&self, session: &PersistedHumanSession) -> HumanSessionResult<Cookie<'static>> {
        let expires = OffsetDateTime::from_unix_timestamp(session.expires_at_unix_ms / 1_000)
            .map_err(|_| HumanSessionFailure::new("desktop.human_session.expiry_invalid", false))?;
        Ok(Cookie::build((
            DESKTOP_SESSION_COOKIE.to_owned(),
            session.session_secret.clone(),
        ))
        .domain(self.cookie_domain()?)
        .path("/")
        .secure(true)
        .http_only(true)
        .same_site(SameSite::None)
        .expires(expires)
        .build())
    }

    fn cookie_identity(&self) -> HumanSessionResult<Cookie<'static>> {
        Ok(Cookie::build(DESKTOP_SESSION_COOKIE.to_owned())
            .domain(self.cookie_domain()?)
            .path("/")
            .secure(true)
            .build())
    }

    fn cookie_domain(&self) -> HumanSessionResult<String> {
        self.control_plane_url
            .host_str()
            .map(str::to_owned)
            .ok_or_else(|| HumanSessionFailure::new("desktop.human_session.url_invalid", false))
    }

    fn cancel_pending_if_current(&self, client_state: &str) -> HumanSessionResult<()> {
        let _guard = self
            .operation_gate
            .lock()
            .map_err(|_| state_unavailable())?;
        if self
            .vault
            .load_pending()?
            .is_some_and(|pending| pending.client_state == client_state)
        {
            self.vault.delete_pending()?;
        }
        Ok(())
    }

    async fn receive_loopback_callback(&self, app: AppHandle, listener: LoopbackCallbackListener) {
        let request = match listener.wait(PENDING_AUTHENTICATION_TTL).await {
            Ok(request) => request,
            Err(failure) => {
                let _ = app.emit(
                    HUMAN_SESSION_FAILED_EVENT,
                    loopback_callback_failure(failure),
                );
                focus_main_window(&app);
                return;
            }
        };
        let callback = parse_loopback_authentication_callback(request.callback_url());
        let authenticated = complete_authentication_callback(&app, self, callback).await;
        let _ = request.respond(authenticated).await;
    }
}

/// 完成回调、广播唯一结果并把桌面窗口带回前台。
pub(crate) async fn complete_authentication_callback(
    app: &AppHandle,
    sessions: &HumanSessionRuntime,
    callback: HumanSessionResult<HumanAuthenticationCallback>,
) -> bool {
    let result = match callback {
        Ok(callback) => sessions.complete_authentication(app, callback).await,
        Err(failure) => Err(failure),
    };
    let authenticated = match result {
        Ok(session) => {
            let _ = app.emit(HUMAN_SESSION_CHANGED_EVENT, session);
            true
        }
        Err(failure) => {
            let _ = app.emit(HUMAN_SESSION_FAILED_EVENT, failure);
            false
        }
    };
    focus_main_window(app);
    authenticated
}

pub(crate) fn authentication_callback(
    url: &Url,
) -> Option<HumanSessionResult<HumanAuthenticationCallback>> {
    if url.scheme() != "agent-room" || url.host_str() != Some("auth") {
        return None;
    }
    Some(parse_authentication_callback(url))
}

fn parse_authentication_callback(url: &Url) -> HumanSessionResult<HumanAuthenticationCallback> {
    if url.username() != ""
        || url.password().is_some()
        || url.port().is_some()
        || url.path() != "/callback"
        || url.fragment().is_some()
    {
        return Err(invalid_callback());
    }
    parse_callback_parameters(url)
}

fn parse_loopback_authentication_callback(
    url: &Url,
) -> HumanSessionResult<HumanAuthenticationCallback> {
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none()
        || url.username() != ""
        || url.password().is_some()
        || url.path() != "/auth/callback"
        || url.fragment().is_some()
    {
        return Err(invalid_callback());
    }
    parse_callback_parameters(url)
}

fn parse_callback_parameters(url: &Url) -> HumanSessionResult<HumanAuthenticationCallback> {
    let mut authorization_code = None;
    let mut client_state = None;
    for (name, value) in url.query_pairs() {
        match name.as_ref() {
            "code" if authorization_code.is_none() => authorization_code = Some(value.into_owned()),
            "state" if client_state.is_none() => client_state = Some(value.into_owned()),
            _ => return Err(invalid_callback()),
        }
    }
    let authorization_code = authorization_code.ok_or_else(invalid_callback)?;
    let client_state = client_state.ok_or_else(invalid_callback)?;
    validate_random_value(&client_state)?;
    if authorization_code.is_empty()
        || authorization_code.len() > MAX_AUTHORIZATION_CODE_LENGTH
        || authorization_code.chars().any(char::is_control)
    {
        return Err(invalid_callback());
    }
    Ok(HumanAuthenticationCallback {
        authorization_code,
        client_state,
    })
}

fn focus_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn validate_pending(
    pending: &PendingAuthentication,
    callback: &HumanAuthenticationCallback,
    now: i64,
) -> HumanSessionResult<()> {
    validate_random_value(&pending.client_state)?;
    validate_random_value(&pending.pkce_verifier)?;
    validate_return_path(&pending.return_path)?;
    if pending.created_at_unix_ms < 0 {
        return Err(state_unavailable());
    }
    let ttl =
        i64::try_from(PENDING_AUTHENTICATION_TTL.as_millis()).map_err(|_| state_unavailable())?;
    let expires_at = pending
        .created_at_unix_ms
        .checked_add(ttl)
        .ok_or_else(state_unavailable)?;
    if now >= expires_at {
        return Err(HumanSessionFailure::new(
            "desktop.human_session.pending_expired",
            false,
        ));
    }
    if callback.client_state != pending.client_state {
        return Err(HumanSessionFailure::new(
            "desktop.human_session.state_mismatch",
            false,
        ));
    }
    Ok(())
}

fn validate_persisted_session(session: &PersistedHumanSession, now: i64) -> HumanSessionResult<()> {
    if session.expires_at_unix_ms <= now
        || session.session_secret.len() < MIN_RANDOM_VALUE_LENGTH
        || session.session_secret.len() > 1_024
        || session.session_secret.chars().any(char::is_control)
    {
        return Err(HumanSessionFailure::new(
            "desktop.human_session.session_invalid",
            false,
        ));
    }
    Ok(())
}

fn validate_return_path(value: &str) -> HumanSessionResult<()> {
    if !value.starts_with('/')
        || value.starts_with("//")
        || value.len() > MAX_RETURN_PATH_LENGTH
        || value.chars().any(char::is_control)
    {
        return Err(HumanSessionFailure::new(
            "desktop.human_session.return_path_invalid",
            false,
        ));
    }
    Ok(())
}

fn random_url_safe_value() -> HumanSessionResult<String> {
    let mut entropy = [0_u8; 32];
    getrandom::fill(&mut entropy)
        .map_err(|_| HumanSessionFailure::new("desktop.human_session.entropy_unavailable", true))?;
    Ok(URL_SAFE_NO_PAD.encode(entropy))
}

fn validate_random_value(value: &str) -> HumanSessionResult<()> {
    if !(MIN_RANDOM_VALUE_LENGTH..=MAX_RANDOM_VALUE_LENGTH).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(invalid_callback());
    }
    Ok(())
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn now_unix_ms() -> HumanSessionResult<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| state_unavailable())?;
    i64::try_from(elapsed.as_millis()).map_err(|_| state_unavailable())
}

fn exchange_failure(status: StatusCode) -> HumanSessionFailure {
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        HumanSessionFailure::new("desktop.human_session.exchange_unavailable", true)
    } else {
        HumanSessionFailure::new("desktop.human_session.exchange_rejected", false)
    }
}

const fn invalid_callback() -> HumanSessionFailure {
    HumanSessionFailure::new("desktop.human_session.callback_invalid", false)
}

const fn unavailable_vault() -> HumanSessionFailure {
    HumanSessionFailure::new("desktop.human_session.vault_unavailable", true)
}

const fn state_unavailable() -> HumanSessionFailure {
    HumanSessionFailure::new("desktop.human_session.state_unavailable", true)
}

const fn loopback_callback_failure(failure: LoopbackCallbackFailure) -> HumanSessionFailure {
    match failure {
        LoopbackCallbackFailure::Timeout => {
            HumanSessionFailure::new("desktop.human_session.loopback_timeout", true)
        }
        LoopbackCallbackFailure::Unavailable => {
            HumanSessionFailure::new("desktop.human_session.loopback_unavailable", true)
        }
    }
}

type HumanSessionResult<TValue> = Result<TValue, HumanSessionFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HumanSessionFailure {
    code: &'static str,
    retryable: bool,
}

impl HumanSessionFailure {
    const fn new(code: &'static str, retryable: bool) -> Self {
        Self { code, retryable }
    }

    pub(crate) const fn code(self) -> &'static str {
        self.code
    }

    pub(crate) const fn retryable(self) -> bool {
        self.retryable
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HumanAuthenticationCallback, PendingAuthentication, authentication_callback,
        parse_authentication_callback, parse_loopback_authentication_callback, pkce_challenge,
        validate_pending, validate_return_path,
    };
    use url::Url;

    #[test]
    fn 桌面回调只接受闭合的授权码与状态参数() {
        let valid = Url::parse(
            "agent-room://auth/callback?code=one-time-code&state=abcdefghijklmnopqrstuvwxyzABCDEF",
        )
        .expect("回调 URL 有效");
        let callback = parse_authentication_callback(&valid).expect("闭合回调有效");
        assert_eq!(callback.authorization_code, "one-time-code");
        assert!(authentication_callback(&valid).is_some());

        for invalid in [
            "agent-room://auth/callback?code=x&state=short",
            "agent-room://auth/callback?code=x&code=y&state=abcdefghijklmnopqrstuvwxyzABCDEF",
            "agent-room://auth/callback?code=x&state=abcdefghijklmnopqrstuvwxyzABCDEF&next=evil",
            "agent-room://auth/other?code=x&state=abcdefghijklmnopqrstuvwxyzABCDEF",
            "agent-room://auth/callback?code=x&state=abcdefghijklmnopqrstuvwxyzABCDEF#fragment",
        ] {
            let url = Url::parse(invalid).expect("测试 URL 可解析");
            assert!(
                parse_authentication_callback(&url).is_err(),
                "应拒绝 {invalid}"
            );
        }
        assert!(
            authentication_callback(&Url::parse("agent-room://lobby/id").expect("大厅 URL 有效"))
                .is_none()
        );
    }

    #[test]
    fn 本机_http_回调必须使用随机回环端口与固定路径() {
        let valid = Url::parse(
            "http://127.0.0.1:49152/auth/callback?code=one-time-code&state=abcdefghijklmnopqrstuvwxyzABCDEF",
        )
        .expect("回环 URL 有效");
        assert!(parse_loopback_authentication_callback(&valid).is_ok());

        for invalid in [
            "http://localhost:49152/auth/callback?code=x&state=abcdefghijklmnopqrstuvwxyzABCDEF",
            "http://127.0.0.1/auth/callback?code=x&state=abcdefghijklmnopqrstuvwxyzABCDEF",
            "http://127.0.0.1:49152/other?code=x&state=abcdefghijklmnopqrstuvwxyzABCDEF",
            "https://127.0.0.1:49152/auth/callback?code=x&state=abcdefghijklmnopqrstuvwxyzABCDEF",
        ] {
            let url = Url::parse(invalid).expect("测试 URL 可解析");
            assert!(
                parse_loopback_authentication_callback(&url).is_err(),
                "应拒绝 {invalid}"
            );
        }
    }

    #[test]
    fn 本地事务同时校验_state_与十五分钟时限() {
        let pending = PendingAuthentication {
            client_state: "abcdefghijklmnopqrstuvwxyzABCDEF".to_owned(),
            pkce_verifier: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ".to_owned(),
            return_path: "/workspace".to_owned(),
            created_at_unix_ms: 1_000,
        };
        let valid = HumanAuthenticationCallback {
            authorization_code: "code".to_owned(),
            client_state: pending.client_state.clone(),
        };
        assert!(validate_pending(&pending, &valid, 1_001).is_ok());

        let mismatch = HumanAuthenticationCallback {
            authorization_code: "code".to_owned(),
            client_state: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef".to_owned(),
        };
        assert_eq!(
            validate_pending(&pending, &mismatch, 1_001)
                .expect_err("状态不匹配必须失败")
                .code(),
            "desktop.human_session.state_mismatch"
        );
        assert_eq!(
            validate_pending(&pending, &valid, 1_000 + 15 * 60 * 1_000 + 1)
                .expect_err("过期事务必须失败")
                .code(),
            "desktop.human_session.pending_expired"
        );
    }

    #[test]
    fn pkce_s256_与返回路径校验不依赖浏览器实现() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert!(validate_return_path("/workspace?tab=agents").is_ok());
        assert!(validate_return_path("//evil.example").is_err());
        assert!(validate_return_path("https://evil.example").is_err());
    }
}
