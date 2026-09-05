use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Manager as _};
use tauri_plugin_opener::OpenerExt as _;
use url::Url;

use crate::{
    authentication_values::{generate_random_url_safe_value, is_valid_return_path},
    desktop_config::DesktopBridgeConfig,
    loopback_callback::{LoopbackCallbackFailure, LoopbackCallbackListener},
};

const AUTHENTICATION_TTL: Duration = Duration::from_mins(15);
const MAX_LOGIN_TOKEN_LENGTH: usize = 4_096;

#[derive(Clone)]
pub(crate) struct MatrixSessionRuntime {
    matrix_base_url: Url,
    authentication_active: Arc<AtomicBool>,
}

impl MatrixSessionRuntime {
    pub(crate) fn system(config: &DesktopBridgeConfig) -> Self {
        Self {
            matrix_base_url: config.matrix_base_url(),
            authentication_active: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) async fn begin_authentication(
        &self,
        app: &AppHandle,
        return_path: &str,
    ) -> MatrixSessionResult<MatrixAuthenticationGrant> {
        if !is_valid_return_path(return_path) {
            return Err(MatrixSessionFailure::new(
                "desktop.matrix_session.return_path_invalid",
                false,
            ));
        }
        let _lease =
            AuthenticationLease::acquire(&self.authentication_active).ok_or_else(|| {
                MatrixSessionFailure::new("desktop.matrix_session.authentication_pending", false)
            })?;
        let result = self.receive_authentication_grant(app, return_path).await;
        focus_main_window(app);
        result
    }

    async fn receive_authentication_grant(
        &self,
        app: &AppHandle,
        return_path: &str,
    ) -> MatrixSessionResult<MatrixAuthenticationGrant> {
        let transaction_id = generate_random_url_safe_value().map_err(|_| {
            MatrixSessionFailure::new("desktop.matrix_session.entropy_unavailable", true)
        })?;
        let listener = LoopbackCallbackListener::bind_matrix_session(&transaction_id)
            .await
            .map_err(|_| {
                MatrixSessionFailure::new("desktop.matrix_session.loopback_bind_failed", true)
            })?;
        let login_url = matrix_sso_url(&self.matrix_base_url, listener.callback_url())?;
        app.opener()
            .open_url(login_url.as_str(), None::<&str>)
            .map_err(|_| {
                MatrixSessionFailure::new("desktop.matrix_session.browser_open_failed", true)
            })?;

        let request = listener
            .wait(AUTHENTICATION_TTL)
            .await
            .map_err(loopback_failure)?;
        let grant = parse_authentication_grant(request.callback_url(), return_path);
        let accepted = grant.is_ok();
        let _ = request.respond(accepted).await;
        grant
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MatrixAuthenticationGrant {
    login_token: String,
    return_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MatrixSessionFailure {
    code: &'static str,
    retryable: bool,
}

impl MatrixSessionFailure {
    pub(crate) const fn new(code: &'static str, retryable: bool) -> Self {
        Self { code, retryable }
    }

    pub(crate) const fn code(self) -> &'static str {
        self.code
    }

    pub(crate) const fn retryable(self) -> bool {
        self.retryable
    }
}

struct AuthenticationLease<'a> {
    active: &'a AtomicBool,
}

impl<'a> AuthenticationLease<'a> {
    fn acquire(active: &'a AtomicBool) -> Option<Self> {
        active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self { active })
    }
}

impl Drop for AuthenticationLease<'_> {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

fn matrix_sso_url(base_url: &Url, callback_url: &Url) -> MatrixSessionResult<Url> {
    let mut login_url = base_url
        .join("_matrix/client/v3/login/sso/redirect")
        .map_err(|_| MatrixSessionFailure::new("desktop.matrix_session.url_invalid", false))?;
    login_url
        .query_pairs_mut()
        .append_pair("redirectUrl", callback_url.as_str());
    Ok(login_url)
}

fn parse_authentication_grant(
    callback_url: &Url,
    return_path: &str,
) -> MatrixSessionResult<MatrixAuthenticationGrant> {
    let mut login_token = None;
    for (name, value) in callback_url.query_pairs() {
        if name == "loginToken" && login_token.is_none() {
            login_token = Some(value.into_owned());
        } else {
            return Err(invalid_callback());
        }
    }
    let login_token = login_token.ok_or_else(invalid_callback)?;
    if login_token.is_empty()
        || login_token.len() > MAX_LOGIN_TOKEN_LENGTH
        || login_token.chars().any(char::is_control)
    {
        return Err(invalid_callback());
    }
    Ok(MatrixAuthenticationGrant {
        login_token,
        return_path: return_path.to_owned(),
    })
}

fn focus_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

const fn invalid_callback() -> MatrixSessionFailure {
    MatrixSessionFailure::new("desktop.matrix_session.callback_invalid", false)
}

const fn loopback_failure(failure: LoopbackCallbackFailure) -> MatrixSessionFailure {
    match failure {
        LoopbackCallbackFailure::Timeout => {
            MatrixSessionFailure::new("desktop.matrix_session.loopback_timeout", true)
        }
        LoopbackCallbackFailure::Unavailable => {
            MatrixSessionFailure::new("desktop.matrix_session.loopback_unavailable", true)
        }
    }
}

type MatrixSessionResult<TValue> = Result<TValue, MatrixSessionFailure>;

#[cfg(test)]
mod tests {
    use super::{matrix_sso_url, parse_authentication_grant};
    use url::Url;

    #[test]
    fn sso_入口只把随机回环地址交给_matrix() {
        let base = Url::parse("https://matrix.agent-room.test/").expect("Matrix 地址有效");
        let callback =
            Url::parse("http://127.0.0.1:45123/matrix/callback/abcdefghijklmnopqrstuvwxyzABCDEF")
                .expect("回环地址有效");

        let login = matrix_sso_url(&base, &callback).expect("SSO 地址可构造");

        assert_eq!(login.path(), "/_matrix/client/v3/login/sso/redirect");
        assert_eq!(
            login
                .query_pairs()
                .find_map(|(name, value)| (name == "redirectUrl").then(|| value.into_owned())),
            Some(callback.to_string())
        );
    }

    #[test]
    fn 回调只接受唯一的一次性登录令牌() {
        let callback =
            Url::parse("http://127.0.0.1:45123/matrix/callback/transaction?loginToken=single-use")
                .expect("回调地址有效");
        let grant = parse_authentication_grant(&callback, "/connect").expect("令牌有效");
        assert_eq!(grant.login_token, "single-use");
        assert_eq!(grant.return_path, "/connect");

        let duplicated = Url::parse(
            "http://127.0.0.1:45123/matrix/callback/transaction?loginToken=one&loginToken=two",
        )
        .expect("重复参数地址可解析");
        assert!(parse_authentication_grant(&duplicated, "/connect").is_err());
    }
}
