use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    ports::{Clock, PortFuture, PublicRoomRecord},
    public_room_directory::{
        BrowsePublicRooms, PublicDirectoryFailure, PublicDirectoryFailureKind,
        PublicDirectoryResult, PublicRoomDetailView, PublicRoomDirectoryUseCases, PublicRoomOnline,
        PublicRoomPage, PublicRoomView,
    },
};
use agent_room_domain::{
    ids::RoomCatalogId,
    room_listing::{RoomJoinMode, RoomShareCode},
    time::UtcMillis,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

use super::{SharePageHttpState, router};
use crate::features::anonymous_limits::{AnonymousLimiter, PER_MINUTE};

const NOW: i64 = 1_790_000_000_000;
const DAY: i64 = 24 * 60 * 60 * 1_000;

/// 分享码 `aB3xYz` 是 `room`，别的码照 `failure` 回答（没设就是没有这间房）。
struct FakeDirectory {
    room: Mutex<PublicRoomDetailView>,
    failure: Mutex<Option<PublicDirectoryFailureKind>>,
    asked: Mutex<Vec<(bool, String)>>,
}

impl FakeDirectory {
    fn new(detail: PublicRoomDetailView) -> Arc<Self> {
        Arc::new(Self {
            room: Mutex::new(detail),
            failure: Mutex::new(None),
            asked: Mutex::new(Vec::new()),
        })
    }
}

impl PublicRoomDirectoryUseCases for FakeDirectory {
    fn browse(
        &self,
        _request: BrowsePublicRooms,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomPage>> {
        Box::pin(async { unreachable!("分享页不翻目录") })
    }

    fn detail(
        &self,
        viewer: Option<AuthenticatedPrincipal>,
        share_code: String,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomDetailView>> {
        let answer = match *self.failure.lock().unwrap() {
            Some(kind) => Err(PublicDirectoryFailure::new("test", kind)),
            None if share_code == "aB3xYz" => Ok(self.room.lock().unwrap().clone()),
            None => Err(PublicDirectoryFailure::new(
                "test",
                PublicDirectoryFailureKind::NotFound,
            )),
        };
        self.asked
            .lock()
            .unwrap()
            .push((viewer.is_some(), share_code));
        Box::pin(async move { answer })
    }
}

struct TestClock;

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(NOW).unwrap()
    }
}

#[tokio::test]
async fn 公开房间出整页_用户写的字都转义_缓存一分钟_不进搜索() {
    let directory = FakeDirectory::new(detail(
        "<script>alert(1)</script> & \"读书会\"",
        "第一行 <img src=x onerror=alert(1)>\n第二行",
        RoomJoinMode::Open,
        false,
    ));
    let (status, headers, html) = get(directory.clone(), "/r/aB3xYz", Some("zh-CN")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=60");
    assert_eq!(headers["x-robots-tag"], "noindex");
    assert_eq!(headers[header::VARY], "Accept-Language");
    assert!(!html.contains("<script>alert"), "{html}");
    assert!(!html.contains("<img"), "{html}");
    assert!(!html.contains("onerror=alert(1)>"), "{html}");
    assert!(html.contains(
        "<meta property=\"og:title\" content=\"&lt;script&gt;alert(1)&lt;/script&gt; &amp; \
         &quot;读书会&quot; · Agent Room\">"
    ));
    assert!(html.contains(
        "<meta property=\"og:description\" content=\"第一行 &lt;img src=x onerror=alert(1)&gt; 第二行\">"
    ));
    assert!(html.contains("<meta name=\"robots\" content=\"noindex\">"));
    assert!(html.contains("content=\"https://app.agent-room.test/room-share-preview-zh-CN.png\""));
    assert!(html.contains("<link rel=\"stylesheet\" href=\"/r/_/share.css\">"));
    assert!(html.contains("<script src=\"/r/_/share.js\" defer></script>"));
    assert!(!html.contains("style=\""), "内容安全策略不许内联样式");
    assert!(html.contains("value=\"https://agent-room.test/r/aB3xYz\""));
    assert_eq!(
        *directory.asked.lock().unwrap(),
        [(false, "aB3xYz".to_owned())],
        "不看登录"
    );
}

#[tokio::test]
async fn 房间的样子_房主_标签_简介_人数_在线_开了多久() {
    let mut room = detail("读书会", "每周读一本书", RoomJoinMode::Open, false);
    room.online = Some(PublicRoomOnline {
        count: 200,
        at_least: true,
    });
    let (_, _, html) = get(FakeDirectory::new(room), "/r/aB3xYz", Some("zh-CN")).await;

    assert!(html.contains("<html lang=\"zh-CN\">"));
    assert!(html.contains("<h1>读书会</h1>"));
    assert!(html.contains("房主 阿青"));
    assert!(html.contains("<li>公开房间</li>"));
    assert!(html.contains("<li class=\"share-tag--open\">直接加入</li>"));
    assert!(html.contains("<li class=\"share-tag--plain\">不加密</li>"));
    assert!(html.contains("这间房不加密：进来的人和服务器都能看到房间里的话。"));
    assert!(html.contains("<p class=\"share-description\">每周读一本书</p>"));
    assert!(html.contains("<div><dt>人</dt><dd>12</dd></div>"));
    assert!(html.contains("<div><dt>Agent</dt><dd>3</dd></div>"));
    assert!(html.contains("<div><dt>现在在线</dt><dd>至少 200</dd></div>"));
    assert!(html.contains("<div><dt>开了</dt><dd>3 天</dd></div>"));
    assert!(html.contains("房间由用户创建，Agent Room 不审核内容。"));

    let encrypted = detail("读书会", "", RoomJoinMode::Open, true);
    let (_, _, html) = get(FakeDirectory::new(encrypted), "/r/aB3xYz", Some("zh-CN")).await;
    assert!(html.contains("<li>端到端加密</li>"));
    assert!(!html.contains("这间房不加密"));
    assert!(!html.contains("share-description"), "没写简介不留空段");
    assert!(!html.contains("现在在线"), "问不到在线的不写");
    assert!(html.contains(
        "<meta property=\"og:description\" content=\"Agent Room 上的公开房间：人和 Agent 在这里一起聊。\">"
    ));
}

#[tokio::test]
async fn 按钮看加入方式_都去网页的发现页() {
    let open = detail("读书会", "", RoomJoinMode::Open, true);
    let (_, _, html) = get(FakeDirectory::new(open), "/r/aB3xYz", Some("zh-CN")).await;
    assert!(html.contains(
        "href=\"https://app.agent-room.test/explore/aB3xYz?join=1\"><span class=\"share-wide\">加入房间</span>\
         <span class=\"share-narrow\">登录并加入</span></a>"
    ));
    assert!(html.contains(
        "<a href=\"https://app.agent-room.test/explore/aB3xYz?join=1&amp;signup=1\">用邮箱注册</a>"
    ));

    let request = detail("读书会", "", RoomJoinMode::Request, true);
    let (_, _, html) = get(FakeDirectory::new(request), "/r/aB3xYz", Some("zh-CN")).await;
    assert!(
        html.contains("href=\"https://app.agent-room.test/explore/aB3xYz?join=1\">申请加入</a>")
    );

    let invite = detail("读书会", "", RoomJoinMode::InviteOnly, true);
    let (_, _, html) = get(FakeDirectory::new(invite), "/r/aB3xYz", Some("zh-CN")).await;
    assert!(html.contains("这个房间只能由房主邀请。"));
    assert!(!html.contains("?join=1"), "进不来就不给按钮");
}

#[tokio::test]
async fn 语言按浏览器_没说的看房间名里有没有中文() {
    for (accept, name, lang) in [
        (Some("en-US,zh;q=0.9"), "读书会", "en"),
        (Some("zh-TW, en;q=0.5"), "Book club", "zh-CN"),
        (Some("fr-FR, zh;q=0.4, de;q=0.8"), "Book club", "zh-CN"),
        (Some("en;q=0.3, zh;q=0.7"), "Book club", "zh-CN"),
        (Some("fr-FR"), "读书会", "zh-CN"),
        (Some("zh;q=0, en"), "读书会", "en"),
        (None, "读书会", "zh-CN"),
        (None, "Book club", "en"),
    ] {
        let room = detail(name, "", RoomJoinMode::Open, true);
        let (_, _, html) = get(FakeDirectory::new(room), "/r/aB3xYz", accept).await;
        assert!(
            html.contains(&format!("<html lang=\"{lang}\">")),
            "{accept:?} {name}"
        );
        assert!(
            html.contains(&format!("/room-share-preview-{lang}.png\"")),
            "预览图跟着页面的语言：{accept:?} {name}"
        );
    }
    let room = detail("Book club", "", RoomJoinMode::Open, true);
    let (_, _, html) = get(FakeDirectory::new(room), "/r/aB3xYz", None).await;
    assert!(html.contains("Hosted by 阿青"));
    assert!(html.contains("<div><dt>Open for</dt><dd>3 days</dd></div>"));
    assert!(html.contains(">Join room</span>"));
}

#[test]
fn 两种语言的预览图都在网页的静态文件里() {
    let public = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/public");
    for language in ["en", "zh-CN"] {
        let image = public.join(format!("room-share-preview-{language}.png"));
        assert!(image.is_file(), "{}", image.display());
    }
}

#[tokio::test]
async fn 不公开了_没有这个码都回一样的404_不写房间名_不缓存() {
    let directory = FakeDirectory::new(detail("读书会", "", RoomJoinMode::Open, true));
    for path in ["/r/Zz9Zz9", "/r/not-a-code", "/r/%3Cscript%3E"] {
        let (status, headers, html) = get(directory.clone(), path, Some("zh-CN")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(headers["x-robots-tag"], "noindex");
        assert!(html.contains("<h1>这个房间现在不公开了</h1>"));
        assert!(!html.contains("读书会"));
        assert!(!html.contains("<script>"), "{path}");
    }

    *directory.failure.lock().unwrap() = Some(PublicDirectoryFailureKind::DependencyUnavailable);
    let (status, headers, html) = get(directory, "/r/aB3xYz", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert!(html.contains("<h1>Can’t open this right now</h1>"));
}

#[tokio::test]
async fn 样式和脚本是两个固定名字的文件() {
    let directory = FakeDirectory::new(detail("读书会", "", RoomJoinMode::Open, true));
    for (path, content_type, marker) in [
        ("/r/_/share.css", "text/css; charset=utf-8", "--ar-canvas"),
        (
            "/r/_/share.js",
            "text/javascript; charset=utf-8",
            "data-copy-link",
        ),
    ] {
        let (status, headers, body) = get(directory.clone(), path, None).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(headers[header::CONTENT_TYPE], content_type);
        assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=300");
        assert!(body.contains(marker), "{path}");
    }
    assert!(
        directory.asked.lock().unwrap().is_empty(),
        "不当成分享码去查"
    );
}

#[tokio::test]
async fn 一个来源一分钟打开太多次就请它等一分钟_样式和脚本不限() {
    let directory = FakeDirectory::new(detail("读书会", "", RoomJoinMode::Open, true));
    let app = test_router(directory.clone());
    let from = |path: &str| {
        Request::builder()
            .uri(path)
            .header("x-forwarded-for", "203.0.113.7")
            .header(header::ACCEPT_LANGUAGE, "zh-CN")
            .body(Body::empty())
            .unwrap()
    };
    for index in 0..PER_MINUTE {
        let response = app.clone().oneshot(from("/r/aB3xYz")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "第 {index} 次");
    }

    let response = app.clone().oneshot(from("/r/aB3xYz")).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let headers = response.headers().clone();
    assert_eq!(headers[header::RETRY_AFTER], "60");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(headers["x-robots-tag"], "noindex");
    let body = to_bytes(response.into_body(), 256 * 1_024).await.unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("<h1>这一会儿打开得太多了</h1>"));
    assert!(!html.contains("读书会"));
    assert_eq!(
        directory.asked.lock().unwrap().len(),
        usize::try_from(PER_MINUTE).unwrap(),
        "超了的那次不去查"
    );

    for path in ["/r/_/share.css", "/r/_/share.js"] {
        let response = app.clone().oneshot(from(path)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }
}

fn test_router(directory: Arc<FakeDirectory>) -> Router {
    router(SharePageHttpState::new(
        directory,
        Arc::new(TestClock),
        Arc::new(AnonymousLimiter::new(Vec::new(), Arc::new(TestClock))),
        "agent-room.test",
        &Url::parse("https://app.agent-room.test").unwrap(),
    ))
}

async fn get(
    directory: Arc<FakeDirectory>,
    path: &str,
    accept_language: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let mut request = Request::builder().uri(path);
    if let Some(value) = accept_language {
        request = request.header(header::ACCEPT_LANGUAGE, value);
    }
    let response = test_router(directory)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 256 * 1_024).await.unwrap();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

fn detail(
    name: &str,
    description: &str,
    join_mode: RoomJoinMode,
    end_to_end_encrypted: bool,
) -> PublicRoomDetailView {
    PublicRoomDetailView {
        room: PublicRoomView {
            room: PublicRoomRecord {
                catalog_id: RoomCatalogId::from_uuid(Uuid::from_u128(10)),
                name: name.to_owned(),
                description: description.to_owned(),
                owner_display_name: "阿青".to_owned(),
                member_count: 12,
                agent_count: 3,
                join_mode,
                end_to_end_encrypted,
                share_code: RoomShareCode::parse("aB3xYz").unwrap(),
                created_at: UtcMillis::new(NOW - 3 * DAY - 1_000).unwrap(),
                last_message_at: None,
            },
            viewer: None,
        },
        admin_display_names: Vec::new(),
        online: None,
    }
}
