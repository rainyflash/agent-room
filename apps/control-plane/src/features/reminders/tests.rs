use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationResult, BeginLogin, CompleteLogin, LoginCompletion,
        LoginRedirect,
    },
    persistence::RepositoryResult,
    ports::{
        Clock, PortFuture, ReminderAccount, ReminderSettingsRepository, ReminderUnsubscribeTokens,
        SecretValue,
    },
    reminders::{ReminderDependencies, ReminderService},
};
use agent_room_domain::{ids::PrincipalId, time::UtcMillis};
use agent_room_identity_adapter::HmacReminderUnsubscribeTokens;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, header},
    middleware,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::*;

const SESSION_COOKIE: &str = "__Host-agent-room-session=session-secret";
const ORIGIN: &str = "https://app.agent-room.test";
const PRINCIPAL: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";

fn principal_id() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::parse_str(PRINCIPAL).unwrap())
}

/// 假的库：存过的开关。
#[derive(Default)]
struct Settings(Mutex<HashMap<PrincipalId, bool>>);

impl ReminderSettingsRepository for Settings {
    fn find_reminder_account(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<ReminderAccount>> {
        let email_enabled = self
            .0
            .lock()
            .unwrap()
            .get(&principal_id)
            .copied()
            .unwrap_or(true);
        Box::pin(async move { Ok(ReminderAccount::Active { email_enabled }) })
    }

    fn set_email_reminders(
        &self,
        principal_id: PrincipalId,
        enabled: bool,
        _changed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        self.0.lock().unwrap().insert(principal_id, enabled);
        Box::pin(async { Ok(true) })
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(1_790_000_000_000).unwrap()
    }
}

struct FakeAuthentication;

impl AuthenticationUseCases for FakeAuthentication {
    fn begin_login(
        &self,
        _request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
        Box::pin(async { unreachable!("提醒设置不会开始登录") })
    }

    fn complete_login<'a>(
        &'a self,
        _request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(async { unreachable!("提醒设置不会完成登录") })
    }

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
        assert_eq!(session_secret.expose(), "session-secret");
        assert_eq!(
            requirement,
            AuthenticationRequirement::ActiveSession,
            "改提醒设置不要最近登录过"
        );
        Box::pin(async {
            Ok(AuthenticatedPrincipal {
                principal_id: principal_id(),
                matrix_user_id: "@rainy:matrix.agent-room.test".to_owned(),
                display_name: "Rainy".to_owned(),
                locale: "zh-CN".to_owned(),
                authenticated_at: UtcMillis::new(1_790_000_000_000).unwrap(),
                expires_at: UtcMillis::new(1_790_000_060_000).unwrap(),
                recently_authenticated: false,
            })
        })
    }

    fn logout<'a>(
        &'a self,
        _session_secret: &'a SecretValue,
    ) -> PortFuture<'a, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("提醒设置不会退出登录") })
    }

    fn suspend_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("提醒设置不会暂停主体") })
    }
}

fn signer() -> HmacReminderUnsubscribeTokens {
    HmacReminderUnsubscribeTokens::new(&[5; 32]).unwrap()
}

fn app(settings: Arc<Settings>, signed: bool) -> Router {
    let service = Arc::new(ReminderService::new(ReminderDependencies {
        repository: settings,
        tokens: signed.then(|| Arc::new(signer()) as Arc<dyn ReminderUnsubscribeTokens>),
        clock: Arc::new(FixedClock),
        email_available: false,
    }));
    router(RemindersHttpState::new(
        service,
        Arc::new(FakeAuthentication),
        Arc::new(AnonymousLimiter::new(Vec::new(), Arc::new(FixedClock))),
        &url::Url::parse(ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn settings_request(method: Method, origin: bool, body: Option<Value>) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri("/account/reminders")
        .header(header::COOKIE, SESSION_COOKIE);
    if origin {
        request = request.header(header::ORIGIN, ORIGIN);
    }
    match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => request.body(Body::empty()).unwrap(),
    }
}

fn page_request(method: Method, uri: &str, form: Option<&str>, language: &str) -> Request<Body> {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::ACCEPT_LANGUAGE, language);
    match form {
        Some(form) => request
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form.to_owned()))
            .unwrap(),
        None => request.body(Body::empty()).unwrap(),
    }
}

async fn body_json(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1_024).await.unwrap()).unwrap()
}

async fn body_text(response: Response) -> String {
    String::from_utf8(
        to_bytes(response.into_body(), 64 * 1_024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

#[tokio::test]
async fn 没登录不给_没存过的照默认开着_服务器没开如实说() {
    let app = app(Arc::default(), true);
    let anonymous = Request::builder()
        .uri("/account/reminders")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(anonymous).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );

    let found = app
        .oneshot(settings_request(Method::GET, false, None))
        .await
        .unwrap();
    assert_eq!(found.status(), StatusCode::OK);
    assert!(
        found.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("no-store")
    );
    assert_eq!(
        body_json(found).await,
        json!({ "email": { "enabled": true, "available": false } })
    );
}

#[tokio::test]
async fn 关上邮件提醒要校验来源和正文() {
    let settings = Arc::new(Settings::default());
    let app = app(settings.clone(), true);
    let body = json!({ "email": { "enabled": false } });
    let foreign = app
        .clone()
        .oneshot(settings_request(Method::PUT, false, Some(body.clone())))
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);
    for wrong in [
        json!({ "email": { "enabled": "no" } }),
        json!({ "email": { "enabled": false, "daily": 2 } }),
        json!({ "push": { "enabled": false } }),
    ] {
        let rejected = app
            .clone()
            .oneshot(settings_request(Method::PUT, true, Some(wrong)))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            body_json(rejected).await["code"],
            "reminders.invalid_request"
        );
    }
    assert!(settings.0.lock().unwrap().is_empty());

    let stored = app
        .clone()
        .oneshot(settings_request(Method::PUT, true, Some(body)))
        .await
        .unwrap();
    assert_eq!(stored.status(), StatusCode::OK);
    assert_eq!(
        body_json(stored).await,
        json!({ "email": { "enabled": false, "available": false } })
    );
    let found = app
        .oneshot(settings_request(Method::GET, false, None))
        .await
        .unwrap();
    assert_eq!(body_json(found).await["email"]["enabled"], false);
}

#[tokio::test]
async fn 退订页打开只看不改_点了按钮才退_再打开是已经退了() {
    let settings = Arc::new(Settings::default());
    let app = app(settings.clone(), true);
    let token = signer().issue(principal_id());
    let uri = format!("/reminders/unsubscribe?token={token}");

    let opened = app
        .clone()
        .oneshot(page_request(Method::GET, &uri, None, "zh-CN,zh;q=0.9"))
        .await
        .unwrap();
    assert_eq!(opened.status(), StatusCode::OK);
    for (name, value) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::CONTENT_SECURITY_POLICY, PAGE_POLICY),
    ] {
        assert_eq!(opened.headers()[&name], value);
    }
    let html = body_text(opened).await;
    assert!(html.contains("<html lang=\"zh-CN\">"));
    assert!(html.contains("不再收提醒邮件？"));
    assert!(html.contains(&format!("name=\"token\" value=\"{token}\"")));
    assert!(html.contains("href=\"https://app.agent-room.test/settings/general\""));
    assert!(!html.contains("<script"));
    assert!(settings.0.lock().unwrap().is_empty(), "打开不算退");

    let done = app
        .clone()
        .oneshot(page_request(
            Method::POST,
            "/reminders/unsubscribe",
            Some(&format!("token={token}")),
            "zh-CN",
        ))
        .await
        .unwrap();
    assert_eq!(done.status(), StatusCode::OK);
    assert!(body_text(done).await.contains("已经退订了"));
    assert_eq!(
        settings.0.lock().unwrap().get(&principal_id()),
        Some(&false)
    );

    let again = app
        .oneshot(page_request(Method::GET, &uri, None, "en"))
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::OK);
    let html = body_text(again).await;
    assert!(html.contains("You’re unsubscribed"));
    assert!(!html.contains("<form"));
}

#[tokio::test]
async fn 邮箱客户端的一键退订_令牌在地址里() {
    let settings = Arc::new(Settings::default());
    let app = app(settings.clone(), true);
    let token = signer().issue(principal_id());
    let done = app
        .oneshot(page_request(
            Method::POST,
            &format!("/reminders/unsubscribe?token={token}"),
            Some("List-Unsubscribe=One-Click"),
            "en",
        ))
        .await
        .unwrap();
    assert_eq!(done.status(), StatusCode::OK);
    assert_eq!(
        settings.0.lock().unwrap().get(&principal_id()),
        Some(&false)
    );
}

#[tokio::test]
async fn 令牌不对_没带令牌_没配钥匙都说链接用不了() {
    let settings = Arc::new(Settings::default());
    let token = signer().issue(principal_id());
    let other = HmacReminderUnsubscribeTokens::new(&[6; 32])
        .unwrap()
        .issue(principal_id());
    for (app, method, uri, form) in [
        (
            app(settings.clone(), true),
            Method::GET,
            "/reminders/unsubscribe".to_owned(),
            None,
        ),
        (
            app(settings.clone(), true),
            Method::GET,
            format!("/reminders/unsubscribe?token={other}"),
            None,
        ),
        (
            app(settings.clone(), true),
            Method::POST,
            "/reminders/unsubscribe".to_owned(),
            Some("List-Unsubscribe=One-Click".to_owned()),
        ),
        (
            app(settings.clone(), false),
            Method::POST,
            "/reminders/unsubscribe".to_owned(),
            Some(format!("token={token}")),
        ),
    ] {
        let response = app
            .oneshot(page_request(method, &uri, form.as_deref(), "en"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        assert!(
            body_text(response)
                .await
                .contains("This unsubscribe link doesn’t work")
        );
    }
    assert!(settings.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 退订页按来源限流() {
    let app = app(Arc::default(), true);
    let mut last = StatusCode::OK;
    for _ in 0..=crate::features::anonymous_limits::PER_MINUTE {
        last = app
            .clone()
            .oneshot(page_request(
                Method::GET,
                "/reminders/unsubscribe",
                None,
                "en",
            ))
            .await
            .unwrap()
            .status();
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
}
