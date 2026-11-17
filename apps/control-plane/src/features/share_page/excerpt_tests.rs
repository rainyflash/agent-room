//! 分享的一段对话那一页（3P-7a-2）：`/r/{分享码}/s/{段码}`。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    ports::PortFuture, public_room_directory::PublicDirectoryFailureKind,
};
use agent_room_domain::{
    room_excerpts::RoomExcerptCode,
    room_listing::{RoomJoinMode, RoomShareCode},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use tower::ServiceExt;
use url::Url;

use super::{
    PAGE_POLICY, SharePageHttpState, router,
    tests::{FakeDirectory, FakeLive, FakeRepositories, NOW, TestClock, detail},
};
use crate::{
    features::anonymous_limits::{AnonymousLimiter, PER_MINUTE},
    public_watch::{
        ExcerptAuthor, ExcerptLine, ExcerptPage, ExcerptPageFailure, RoomExcerptPageSource,
        ShownExcerpt,
    },
};

const PATH: &str = "/r/aB3xYz/s/Seg8Ab3x";

type Answer = Result<Arc<ExcerptPage>, ExcerptPageFailure>;

/// 分享码 `aB3xYz` 的段 `Seg8Ab3x` 照 `answer` 回答，别的都没有；记下问过哪些。
struct FakeExcerpts {
    answer: Mutex<Answer>,
    asked: Mutex<Vec<(String, String)>>,
}

impl FakeExcerpts {
    fn answering(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(answer),
            asked: Mutex::new(Vec::new()),
        })
    }
}

impl RoomExcerptPageSource for FakeExcerpts {
    fn page<'a>(
        &'a self,
        share_code: &'a RoomShareCode,
        code: &'a RoomExcerptCode,
    ) -> PortFuture<'a, Answer> {
        self.asked
            .lock()
            .unwrap()
            .push((share_code.as_str().to_owned(), code.as_str().to_owned()));
        let answer = if (share_code.as_str(), code.as_str()) == ("aB3xYz", "Seg8Ab3x") {
            self.answer.lock().unwrap().clone()
        } else {
            Err(ExcerptPageFailure::NotFound)
        };
        Box::pin(async move { answer })
    }
}

fn app(excerpts: Arc<FakeExcerpts>) -> Router {
    router(
        SharePageHttpState::new(
            FakeDirectory::new(detail("读书会", "", RoomJoinMode::Open, false)),
            FakeRepositories::answering(Err(PublicDirectoryFailureKind::NotFound)),
            Arc::new(FakeLive::default()),
            Arc::new(TestClock),
            Arc::new(AnonymousLimiter::new(Vec::new(), Arc::new(TestClock))),
            "agent-room.test",
            &Url::parse("https://app.agent-room.test").unwrap(),
        )
        .with_excerpts(excerpts),
    )
}

async fn get(app: &Router, path: &str, language: Option<&str>) -> (StatusCode, HeaderMap, String) {
    let mut request = Request::builder()
        .uri(path)
        .header("x-forwarded-for", "203.0.113.9");
    if let Some(language) = language {
        request = request.header(header::ACCEPT_LANGUAGE, language);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 256 * 1_024).await.unwrap();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

fn line(name: &str, author: ExcerptAuthor, text: &str, seconds_ago: i64) -> ExcerptLine {
    ExcerptLine {
        name: name.to_owned(),
        author,
        text: text.to_owned(),
        truncated: false,
        withheld: false,
        attachment: false,
        edited: false,
        sent_at_unix_ms: NOW - seconds_ago * 1_000,
    }
}

/// 这一段看得到：房间名和这几句。
fn showing(room_name: &str, lines: Vec<ExcerptLine>) -> Arc<FakeExcerpts> {
    FakeExcerpts::answering(Ok(Arc::new(ExcerptPage::Shown(ShownExcerpt {
        room_name: room_name.to_owned(),
        lines,
    }))))
}

/// 一段四句：人、有主人的本机 Agent（太长了）、网络 Agent（不公开的）、没主人的本机 Agent（改过、带文件）。
fn four_lines() -> Vec<ExcerptLine> {
    let mut long = line(
        "Scout #2316",
        ExcerptAuthor::Agent {
            owner: Some("<b>阿明</b>".to_owned()),
        },
        "很长的开头",
        80,
    );
    long.truncated = true;
    let mut withheld = line("Ranger #1000", ExcerptAuthor::NetworkAgent, "", 70);
    withheld.withheld = true;
    let mut edited = line(
        "Echo #0001",
        ExcerptAuthor::Agent { owner: None },
        "改过的话",
        60,
    );
    edited.edited = true;
    edited.attachment = true;
    vec![
        line(
            "小雨",
            ExcerptAuthor::Person,
            "<script>alert(1)</script>",
            90,
        ),
        long,
        withheld,
        edited,
    ]
}

#[tokio::test]
async fn 一段对话出整页_名字和话都转义_缓存一分钟_不进搜索() {
    let excerpts = showing("<i>读书会</i>", four_lines());
    let app = app(excerpts.clone());

    let (status, headers, html) = get(&app, PATH, Some("zh-CN")).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=60");
    assert_eq!(headers["x-robots-tag"], "noindex");
    assert_eq!(headers[header::CONTENT_SECURITY_POLICY], PAGE_POLICY);
    assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");
    assert!(html.contains("<html lang=\"zh-CN\">"));
    assert!(html.contains("<title>一段对话 · &lt;i&gt;读书会&lt;/i&gt; · Agent Room</title>"));
    assert!(html.contains("<h1>&lt;i&gt;读书会&lt;/i&gt;</h1>"));
    assert!(html.contains(
        "一段对话 · 4 句 · <time datetime=\"2026-09-21\" data-local-date=\"1789999910000\">\
         9 月 21 日</time>"
    ));
    assert!(html.contains("<span class=\"share-live-name\">小雨</span>"));
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(!html.contains("<script>alert"), "{html}");
    assert!(html.contains(
        "<time class=\"share-live-time\" datetime=\"2026-09-21T14:11Z\" \
         data-local-time=\"1789999910000\">14:11</time>"
    ));
    assert!(
        html.contains("<span class=\"share-live-kind\">&lt;b&gt;阿明&lt;/b&gt; 的 Agent</span>")
    );
    assert!(html.contains("很长的开头（太长了，只显示开头）"));
    assert!(html.contains("<span class=\"share-live-kind\">网络 Agent</span>"));
    assert!(
        html.contains("<p class=\"share-live-text share-live-text--quiet\">一条不公开的消息</p>")
    );
    assert!(html.contains("<span class=\"share-live-kind\">Agent</span>"));
    assert!(html.contains("改过的话（改过）"));
    assert!(html.contains("发了一个文件"));
    assert!(html.contains(
        "<a class=\"share-button share-button--primary\" \
         href=\"https://agent-room.test/r/aB3xYz\">去看这间房</a>"
    ));
    assert!(html.contains("房间由用户创建，Agent Room 不审核内容。"));
    assert!(!html.contains("style=\""), "内容安全策略不许内联样式");
    assert!(!html.contains("<script>"), "不许内联脚本");
    assert!(html.contains("<script src=\"/r/_/share.js\" defer></script>"));
    assert_eq!(
        *excerpts.asked.lock().unwrap(),
        [("aB3xYz".to_owned(), "Seg8Ab3x".to_owned())]
    );
}

#[tokio::test]
async fn 读不到了回_410_写着看不了了_能回分享页() {
    let excerpts = FakeExcerpts::answering(Ok(Arc::new(ExcerptPage::Gone {
        room_name: "<i>读书会</i>".to_owned(),
    })));
    let (status, headers, html) = get(&app(excerpts), PATH, Some("zh-CN")).await;

    assert_eq!(status, StatusCode::GONE);
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=60");
    assert_eq!(headers["x-robots-tag"], "noindex");
    assert_eq!(headers[header::CONTENT_SECURITY_POLICY], PAGE_POLICY);
    assert!(html.contains("<h1>这段对话现在看不了了</h1>"));
    assert!(html.contains("&lt;i&gt;读书会&lt;/i&gt;"));
    assert!(!html.contains("<i>读书会"));
    assert!(html.contains("href=\"https://agent-room.test/r/aB3xYz\">去看这间房</a>"));

    let excerpts = FakeExcerpts::answering(Ok(Arc::new(ExcerptPage::Gone {
        room_name: "Book club".to_owned(),
    })));
    let (status, _, html) = get(&app(excerpts), PATH, None).await;
    assert_eq!(status, StatusCode::GONE);
    assert!(html.contains("<h1>This conversation can’t be shown anymore</h1>"));
    assert!(html.contains(">See the room</a>"));
}

#[tokio::test]
async fn 不放实况_码不对_没有这一段都和分享页没有这个码一模一样() {
    let excerpts = FakeExcerpts::answering(Err(ExcerptPageFailure::NotFound));
    let app = app(excerpts.clone());
    let (share_status, share_headers, share_html) = get(&app, "/r/Zz9Zz9", Some("zh-CN")).await;
    assert_eq!(share_status, StatusCode::NOT_FOUND);

    for path in [
        PATH,
        "/r/aB3xYz/s/Zz9Zz9Zz",
        "/r/Zz9Zz9/s/Seg8Ab3x",
        "/r/aB3xYz/s/not-a-code",
        "/r/not-a-code/s/Seg8Ab3x",
        "/r/aB3xYz/s/%3Cscript%3E",
    ] {
        let (status, headers, html) = get(&app, path, Some("zh-CN")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(html, share_html, "{path}");
        for name in [
            header::CACHE_CONTROL,
            header::CONTENT_TYPE,
            header::CONTENT_SECURITY_POLICY,
            header::VARY,
        ] {
            assert_eq!(headers[&name], share_headers[&name], "{path} {name}");
        }
        assert_eq!(headers["x-robots-tag"], "noindex");
        assert!(!html.contains("读书会"), "{path}");
    }
    assert_eq!(
        excerpts.asked.lock().unwrap().len(),
        3,
        "码写得不对的不去查"
    );
}

#[tokio::test]
async fn 暂时读不到回_503_不缓存() {
    let excerpts = FakeExcerpts::answering(Err(ExcerptPageFailure::Unavailable));
    let (status, headers, html) = get(&app(excerpts), PATH, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert!(html.contains("<h1>Can’t open this right now</h1>"));
}

#[tokio::test]
async fn 语言按浏览器_没说的看房间名里有没有中文() {
    let lines = vec![
        line("Scout", ExcerptAuthor::Person, "hi", 20),
        line("Ranger", ExcerptAuthor::Person, "hello", 10),
    ];
    let english = app(showing("读书会", lines.clone()));
    let (_, _, html) = get(&english, PATH, Some("en-US,zh;q=0.5")).await;
    assert!(html.contains("<html lang=\"en\">"));
    assert!(html.contains("A conversation · 2 messages · <time"));
    assert!(html.contains(">Sep 21</time>"));
    assert!(html.contains(">See the room</a>"));

    let (_, _, html) = get(&english, PATH, None).await;
    assert!(html.contains("<html lang=\"zh-CN\">"), "房间名里有中文");

    let book_club = app(showing("Book club", lines[..1].to_vec()));
    let (_, _, html) = get(&book_club, PATH, None).await;
    assert!(html.contains("A conversation · 1 message · <time"));
}

#[tokio::test]
async fn 不是今年的写上年份() {
    let mut old = line("小雨", ExcerptAuthor::Person, "去年的话", 0);
    old.sent_at_unix_ms = 1_760_000_000_000;
    let app = app(showing("读书会", vec![old]));
    let (_, _, html) = get(&app, PATH, Some("zh")).await;
    assert!(html.contains(">2025 年 10 月 9 日</time>"), "{html}");
    let (_, _, html) = get(&app, PATH, Some("en")).await;
    assert!(html.contains(">Oct 9, 2025</time>"));
}

#[tokio::test]
async fn 一个来源一分钟打开太多次就请它等一分钟_和分享页算在一起() {
    let excerpts = showing("读书会", four_lines());
    let app = app(excerpts.clone());
    for index in 0..PER_MINUTE {
        let (status, _, _) = get(&app, PATH, Some("zh-CN")).await;
        assert_eq!(status, StatusCode::OK, "第 {index} 次");
    }
    let (status, headers, html) = get(&app, "/r/aB3xYz", Some("zh-CN")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "分享页也满了");
    assert_eq!(headers[header::RETRY_AFTER], "60");

    let (status, _, html_excerpt) = get(&app, PATH, Some("zh-CN")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(html_excerpt, html);
    assert!(!html.contains("读书会"));
    assert_eq!(
        excerpts.asked.lock().unwrap().len(),
        usize::try_from(PER_MINUTE).unwrap(),
        "超了的那次不去查"
    );
}

#[tokio::test]
async fn 没接上一段对话时哪一段都没有() {
    let app = router(SharePageHttpState::new(
        FakeDirectory::new(detail("读书会", "", RoomJoinMode::Open, false)),
        FakeRepositories::answering(Err(PublicDirectoryFailureKind::NotFound)),
        Arc::new(FakeLive::default()),
        Arc::new(TestClock),
        Arc::new(AnonymousLimiter::new(Vec::new(), Arc::new(TestClock))),
        "agent-room.test",
        &Url::parse("https://app.agent-room.test").unwrap(),
    ));
    let (status, _, _) = get(&app, PATH, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
