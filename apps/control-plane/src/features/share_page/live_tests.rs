//! 分享页放实况（3P-6a-2）：页面上那一块、`/r/{分享码}/live` 和这几页自己带的内容安全策略。

use std::sync::Arc;

use agent_room_application::public_room_directory::PublicDirectoryFailureKind;
use agent_room_domain::room_listing::RoomJoinMode;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use serde_json::Value;
use tower::ServiceExt;
use url::Url;

use super::{
    PAGE_POLICY, SharePageHttpState, router,
    tests::{FakeDirectory, FakeLive, FakeRepositories, TestClock, detail},
};
use crate::{
    features::anonymous_limits::{AnonymousLimiter, PER_MINUTE},
    public_watch::RoomLiveFailure,
};

const SNAPSHOT: &str = r#"{"schemaVersion":1,"messages":[]}"#;

fn app(
    directory: Arc<FakeDirectory>,
    repositories: Arc<FakeRepositories>,
    live: Arc<FakeLive>,
) -> Router {
    router(SharePageHttpState::new(
        directory,
        repositories,
        live,
        Arc::new(TestClock),
        Arc::new(AnonymousLimiter::new(Vec::new(), Arc::new(TestClock))),
        "agent-room.test",
        &Url::parse("https://app.agent-room.test").unwrap(),
    ))
}

fn open_room() -> Arc<FakeDirectory> {
    FakeDirectory::new(detail("读书会", "", RoomJoinMode::Open, false))
}

fn unclaimed() -> Arc<FakeRepositories> {
    FakeRepositories::answering(Err(PublicDirectoryFailureKind::NotFound))
}

async fn get(app: &Router, path: &str) -> (StatusCode, HeaderMap, String) {
    let request = Request::builder()
        .uri(path)
        .header(header::ACCEPT_LANGUAGE, "zh-CN")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 256 * 1_024).await.unwrap();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

fn error_code(body: &str) -> String {
    let value: Value = serde_json::from_str(body).unwrap();
    value["code"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn 放着实况的房间_卡片下面多一块_先藏着由脚本填() {
    let live = FakeLive::showing("aB3xYz", Ok(SNAPSHOT));
    let app = app(open_room(), unclaimed(), live.clone());

    let (status, headers, html) = get(&app, "/r/aB3xYz").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=60");
    assert!(
        html.contains(
            "<section class=\"share-panel share-live\" hidden data-live=\"/r/aB3xYz/live\""
        )
    );
    assert!(html.contains("<h2>房间里正在说的话</h2>"));
    assert!(html.contains("data-online=\"现在 {count} 个在线 · 只读\""));
    assert!(html.contains("data-agent-of=\"{owner} 的 Agent\""));
    assert!(html.contains("data-file=\"发了一个文件\""));
    assert!(html.contains("<p class=\"share-live-join\">加入房间就能说话。</p>"));
    assert!(!html.contains("style=\""), "内容安全策略不许内联样式");
    assert!(!html.contains("<script>"), "不许内联脚本");
    // 实况在房间卡片和“带上你的 Agent”之间。
    let live_at = html.find("share-live").unwrap();
    assert!(html.find("<h1>").unwrap() < live_at);
    assert!(live_at < html.find("带上你的 Agent").unwrap());
    assert_eq!(*live.asked.lock().unwrap(), ["aB3xYz"]);
}

#[tokio::test]
async fn 不放实况的_要申请的_加密的房间页面上没有那一块_后两种不去问() {
    let live = Arc::new(FakeLive::default());
    let quiet = app(open_room(), unclaimed(), live.clone());
    let (_, _, html) = get(&quiet, "/r/aB3xYz").await;
    assert!(!html.contains("data-live"), "{html}");
    assert_eq!(*live.asked.lock().unwrap(), ["aB3xYz"]);

    for room in [
        detail("读书会", "", RoomJoinMode::Request, false),
        detail("读书会", "", RoomJoinMode::Open, true),
    ] {
        let live = FakeLive::showing("aB3xYz", Ok(SNAPSHOT));
        let shown = app(FakeDirectory::new(room), unclaimed(), live.clone());
        let (_, _, html) = get(&shown, "/r/aB3xYz").await;
        assert!(!html.contains("data-live"), "{html}");
        assert!(live.asked.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn 仓库地址的分享页也放实况() {
    let live = FakeLive::showing("aB3xYz", Ok(SNAPSHOT));
    let repositories = FakeRepositories::answering(Ok(detail(
        "rainyflash/agent-room",
        "",
        RoomJoinMode::Open,
        false,
    )));
    let app = app(open_room(), repositories, live);

    let (status, _, html) = get(&app, "/gh/rainyflash/agent-room").await;

    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("data-live=\"/r/aB3xYz/live\""));
}

#[tokio::test]
async fn 实况放着的回快照_缓存三秒_不进搜索() {
    let app = app(
        open_room(),
        unclaimed(),
        FakeLive::showing("aB3xYz", Ok(SNAPSHOT)),
    );

    let (status, headers, body) = get(&app, "/r/aB3xYz/live").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, SNAPSHOT);
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=3");
    assert_eq!(headers["x-robots-tag"], "noindex");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
}

#[tokio::test]
async fn 不放的_没有的_码不对的都回同一个_404_读不到的回_503() {
    let live = FakeLive::showing("aB3xYz", Err(RoomLiveFailure::Unavailable));
    let app = app(open_room(), unclaimed(), live.clone());

    for path in ["/r/zzzzzz/live", "/r/abc/live", "/r/aB3xY!/live"] {
        let (status, headers, body) = get(&app, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(error_code(&body), "room_live.unavailable", "{path}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{path}");
    }
    assert_eq!(
        *live.asked.lock().unwrap(),
        ["zzzzzz"],
        "码写得不对的不去查"
    );

    let (status, headers, body) = get(&app, "/r/aB3xYz/live").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code(&body), "room_live.dependency_unavailable");
    assert_eq!(headers[header::RETRY_AFTER], "5");
}

#[tokio::test]
async fn 实况只有没查过的码才算来源的名额() {
    let live = FakeLive::showing("aB3xYz", Ok(SNAPSHOT));
    let app = app(open_room(), unclaimed(), live.clone());

    *live.known.lock().unwrap() = true;
    for _ in 0..=PER_MINUTE {
        let (status, _, _) = get(&app, "/r/aB3xYz/live").await;
        assert_eq!(status, StatusCode::OK, "刚查过的码不占名额");
    }

    *live.known.lock().unwrap() = false;
    for _ in 0..PER_MINUTE {
        let (status, _, _) = get(&app, "/r/aB3xYz/live").await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, headers, body) = get(&app, "/r/aB3xYz/live").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error_code(&body), "room_live.rate_limited");
    assert!(headers.contains_key(header::RETRY_AFTER));
}

#[tokio::test]
async fn 这几页都带自己的内容安全策略_比网站多一句_connect_src() {
    assert!(PAGE_POLICY.contains("connect-src 'self'"));
    assert!(PAGE_POLICY.contains("script-src 'self'"));
    assert!(!PAGE_POLICY.contains("unsafe-inline"));
    let app = app(
        open_room(),
        unclaimed(),
        FakeLive::showing("aB3xYz", Ok(SNAPSHOT)),
    );
    for path in [
        "/r/aB3xYz",
        "/r/aB3xYz/live",
        "/r/zzzzzz",
        "/r/zzzzzz/live",
        "/r/_/share.css",
        "/r/_/share.js",
        "/gh/rainyflash/agent-room",
        "/gh/rainyflash/agent-room/badge.svg",
        "/gh/-bad-/name",
    ] {
        let (_, headers, _) = get(&app, path).await;
        assert_eq!(
            headers
                .get_all(header::CONTENT_SECURITY_POLICY)
                .iter()
                .count(),
            1,
            "{path}"
        );
        assert_eq!(
            headers[header::CONTENT_SECURITY_POLICY],
            PAGE_POLICY,
            "{path}"
        );
    }
}
