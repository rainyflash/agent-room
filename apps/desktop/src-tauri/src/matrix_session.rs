use std::{sync::Arc, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Manager as _};
use tauri_plugin_opener::OpenerExt as _;
use tokio::sync::watch;
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
    attempts: AuthenticationAttempts,
}

impl MatrixSessionRuntime {
    pub(crate) fn system(config: &DesktopBridgeConfig) -> Self {
        Self {
            matrix_base_url: config.matrix_base_url(),
            attempts: AuthenticationAttempts::default(),
        }
    }

    /// 打开系统浏览器去 Matrix 登录，等回环回调带回登录令牌。
    ///
    /// 同一时间只等一次。又开始一次登录时（人关掉了登录页、点了“重新开始登录”），还在等的那次
    /// 立刻以 `authentication_superseded` 结束、放掉它的回环端口，由新的一次接手，不用等它超时。
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
        let mut attempt = self.attempts.start();
        let outcome = tokio::select! {
            result = self.receive_authentication_grant(app, return_path) => Some(result),
            () = attempt.superseded() => None,
        };
        let Some(result) = outcome else {
            return Err(MatrixSessionFailure::new(
                "desktop.matrix_session.authentication_superseded",
                true,
            ));
        };
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
            })?
            .with_language(crate::native_language::language(app));
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

/// 第几次登录：每开始一次加一，还在等的那次看到它变了就让位给新的一次。
#[derive(Clone)]
struct AuthenticationAttempts {
    generation: Arc<watch::Sender<u64>>,
}

impl Default for AuthenticationAttempts {
    fn default() -> Self {
        Self {
            generation: Arc::new(watch::channel(0).0),
        }
    }
}

impl AuthenticationAttempts {
    fn start(&self) -> AuthenticationAttempt {
        let mut generation = 0;
        self.generation.send_modify(|value| {
            *value = value.wrapping_add(1);
            generation = *value;
        });
        AuthenticationAttempt {
            generation,
            newer: self.generation.subscribe(),
        }
    }
}

struct AuthenticationAttempt {
    generation: u64,
    newer: watch::Receiver<u64>,
}

impl AuthenticationAttempt {
    /// 又开始了更新的一次登录时返回；没有就一直等下去，由调用方自己的超时收尾。
    async fn superseded(&mut self) {
        while self.newer.changed().await.is_ok() {
            if *self.newer.borrow_and_update() != self.generation {
                return;
            }
        }
        std::future::pending::<()>().await;
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
    use std::time::Duration;

    use super::{AuthenticationAttempts, matrix_sso_url, parse_authentication_grant};
    use url::Url;

    #[tokio::test(start_paused = true)]
    async fn 又开始一次登录时还在等的那次立刻让位_最新的一次接着等() {
        let attempts = AuthenticationAttempts::default();
        let mut first = attempts.start();
        let mut second = attempts.start();

        tokio::time::timeout(Duration::from_secs(1), first.superseded())
            .await
            .expect("旧的一次应当让位");
        assert!(
            tokio::time::timeout(Duration::from_mins(1), second.superseded())
                .await
                .is_err(),
            "最新的一次不该让位"
        );

        let mut third = attempts.start();
        tokio::time::timeout(Duration::from_secs(1), second.superseded())
            .await
            .expect("第三次开始后第二次也让位");
        assert!(
            tokio::time::timeout(Duration::from_mins(1), third.superseded())
                .await
                .is_err()
        );
    }

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
