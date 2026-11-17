//! 仓库地址 `/gh/{owner}/{repo}` 和 README 徽章（3P-3）。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    ports::Clock,
    public_room_directory::{PublicDirectoryFailureKind, PublicRoomDetailView, PublicRoomOnline},
};
use agent_room_domain::{room_listing::RoomJoinMode, time::UtcMillis};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use tower::ServiceExt;
use url::Url;

use super::{
    SharePageHttpState,
    badge::{COUNT_TTL_MILLIS, NEW_PER_SECOND, NO_ROOM_TTL_MILLIS},
    router,
    tests::{FakeDirectory, FakeLive, FakeRepositories, NOW, detail},
};
use crate::features::anonymous_limits::{AnonymousLimiter, PER_MINUTE};

/// 能往前拨的钟。
struct MovingClock(Mutex<i64>);

impl MovingClock {
    fn advance(&self, millis: i64) {
        *self.0.lock().unwrap() += millis;
    }
}

impl Clock for MovingClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(*self.0.lock().unwrap()).unwrap()
    }
}

struct Fixture {
    app: Router,
    directory: Arc<FakeDirectory>,
    repositories: Arc<FakeRepositories>,
    clock: Arc<MovingClock>,
}

impl Fixture {
    fn new(repositories: Arc<FakeRepositories>) -> Self {
        let room = detail("Book club", "", RoomJoinMode::Open, false);
        let directory = FakeDirectory::new(room);
        let clock = Arc::new(MovingClock(Mutex::new(NOW)));
        let app = router(SharePageHttpState::new(
            directory.clone(),
            repositories.clone(),
            Arc::new(FakeLive::default()),
            clock.clone(),
            Arc::new(AnonymousLimiter::new(Vec::new(), clock.clone())),
            "agent-room.test",
            &Url::parse("https://app.agent-room.test").unwrap(),
        ));
        Self {
            app,
            directory,
            repositories,
            clock,
        }
    }

    async fn get(&self, path: &str, headers: &[(&str, &str)]) -> (StatusCode, HeaderMap, String) {
        let mut request = Request::builder()
            .uri(path)
            .header("x-forwarded-for", "203.0.113.9");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), 256 * 1_024).await.unwrap();
        (status, headers, String::from_utf8(body.to_vec()).unwrap())
    }
}

fn online_room(name: &str, online: u32) -> PublicRoomDetailView {
    let mut room = detail(name, "", RoomJoinMode::Open, false);
    room.online = Some(PublicRoomOnline {
        count: online,
        at_least: false,
    });
    room
}

#[tokio::test]
async fn 名字不合规矩的直接404_不查库_不把地址里的字写回页面() {
    let fixture = Fixture::new(FakeRepositories::answering(Ok(online_room("Book club", 7))));
    for path in [
        "/gh/-octo/hello",
        "/gh/octo/..",
        "/gh/oc%20to/hello",
        "/gh/octo/%3Cscript%3E",
        "/gh/octo_cat/hello",
        "/gh/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/hello",
    ] {
        let (status, headers, html) = fixture.get(path, &[("accept-language", "zh-CN")]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{path}");
        assert_eq!(headers["x-robots-tag"], "noindex", "{path}");
        assert!(html.contains("<h1>找不到这个页面</h1>"), "{path}");
        assert!(
            !html.contains("script%3E") && !html.contains("<script>"),
            "{path}"
        );
        assert!(!html.contains("octo"), "{path}");

        let (status, _, _) = fixture.get(&format!("{path}/badge.svg"), &[]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    assert!(fixture.repositories.asked().is_empty(), "一次也没查");
}

#[tokio::test]
async fn 绑着公开着的房间出它的分享页_和r地址同一页_大小写不论() {
    let fixture = Fixture::new(FakeRepositories::answering(Ok(online_room("Book club", 3))));
    let (status, headers, html) = fixture
        .get("/gh/Octo-Cat/Hello.World", &[("accept-language", "zh-CN")])
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=60");
    assert_eq!(headers["x-robots-tag"], "noindex");
    assert_eq!(headers[header::VARY], "Accept-Language");
    assert_eq!(fixture.repositories.asked(), ["Octo-Cat/Hello.World"]);

    // 目录里同一间房的 `/r/<分享码>`：一个字都不差（在线数也一样）。
    *fixture.directory.room.lock().unwrap() = online_room("Book club", 3);
    let (_, _, share) = fixture
        .get("/r/aB3xYz", &[("accept-language", "zh-CN")])
        .await;
    assert_eq!(html, share);
    assert!(html.contains("<h1>Book club</h1>"));
    assert!(html.contains("value=\"https://agent-room.test/r/aB3xYz\""));
}

#[tokio::test]
async fn 没有房间出同一页还没人认领_回404_不分是哪一种() {
    let fixture = Fixture::new(FakeRepositories::answering(Err(
        PublicDirectoryFailureKind::NotFound,
    )));
    let (status, headers, html) = fixture
        .get("/gh/octo/hello", &[("accept-language", "zh-CN")])
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=60");
    assert_eq!(headers["x-robots-tag"], "noindex");
    assert!(html.contains("<meta name=\"robots\" content=\"noindex\">"));
    assert!(html.contains("<p class=\"share-mono\">agent-room.test/gh/octo/hello</p>"));
    assert!(html.contains("<h1 class=\"share-repo\"><span>octo/</span>hello</h1>"));
    assert!(html.contains("这个仓库还没有 Agent Room 房间。"));
    assert!(html.contains("你是维护者吗？一条命令建一个"));
    assert!(html.contains("value=\"agent-room project\""));
    assert!(html.contains("data-copy-link=\"project-command\""));
    assert!(html.contains("value=\"给这个仓库建一间 Agent Room 房间。\""));
    assert!(html.contains("data-copy-link=\"project-ask\""));
    assert!(html.contains(
        "<a class=\"share-button\" href=\"https://github.com/rainyflash/agent-room/releases/latest\">下载 Mac 版</a>"
    ));
    assert!(html.contains("下载 Windows 版"));
    assert!(html.contains("在房间设置的“公开”里绑上这个仓库"));
    assert!(html.contains("<a href=\"https://app.agent-room.test/explore\">看看别的公开房间</a>"));
    assert!(html.contains("<link rel=\"stylesheet\" href=\"/r/_/share.css\">"));
    assert!(html.contains("<script src=\"/r/_/share.js\" defer></script>"));
    assert!(!html.contains("style=\""), "内容安全策略不许内联样式");
    assert!(!html.contains("<script>"), "内容安全策略不许内联脚本");
    assert!(!html.contains("Book club"));

    // 应用层把“没有”的几种都说成 NotFound；万一说成 InvalidRequest，也是同一页。
    *fixture.repositories.answer.lock().unwrap() = Err(PublicDirectoryFailureKind::InvalidRequest);
    let (status, _, again) = fixture
        .get("/gh/octo/hello", &[("accept-language", "zh-CN")])
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(again, html);

    *fixture.repositories.answer.lock().unwrap() =
        Err(PublicDirectoryFailureKind::DependencyUnavailable);
    let (status, headers, html) = fixture.get("/gh/octo/hello", &[]).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert!(html.contains("<h1>Can’t open this right now</h1>"));
}

#[tokio::test]
async fn 语言照分享页的规则() {
    let fixture = Fixture::new(FakeRepositories::answering(Err(
        PublicDirectoryFailureKind::NotFound,
    )));
    for (accept, lang, lead) in [
        (None, "en", "There’s no Agent Room for this repository yet."),
        (
            Some("en-US,zh;q=0.9"),
            "en",
            "There’s no Agent Room for this repository yet.",
        ),
        (
            Some("zh-TW, en;q=0.5"),
            "zh-CN",
            "这个仓库还没有 Agent Room 房间。",
        ),
        (
            Some("fr-FR"),
            "en",
            "There’s no Agent Room for this repository yet.",
        ),
    ] {
        let headers: Vec<(&str, &str)> = accept
            .map(|value| ("accept-language", value))
            .into_iter()
            .collect();
        let (_, _, html) = fixture.get("/gh/octo/hello", &headers).await;
        assert!(
            html.contains(&format!("<html lang=\"{lang}\">")),
            "{accept:?}"
        );
        assert!(html.contains(lead), "{accept:?}");
    }
    let (_, _, html) = fixture.get("/gh/octo/hello", &[]).await;
    assert!(html.contains("Maintain this repository? Set one up with one command"));
    assert!(html.contains("value=\"Set up an Agent Room for this repository.\""));
    assert!(html.contains("Bind this repository under “Public” in the room’s settings."));

    // 绑着房间的照分享页：没说语言时看房间名里有没有中文。
    *fixture.repositories.answer.lock().unwrap() = Ok(online_room("读书会", 1));
    let (_, _, html) = fixture.get("/gh/octo/hello", &[]).await;
    assert!(html.contains("<html lang=\"zh-CN\">"));
}

#[tokio::test]
async fn 页面和分享页合起来按来源限流_名字不合规矩的和徽章不算() {
    let fixture = Fixture::new(FakeRepositories::answering(Err(
        PublicDirectoryFailureKind::NotFound,
    )));
    for index in 0..PER_MINUTE {
        let path = if index % 2 == 0 {
            "/gh/octo/hello"
        } else {
            "/r/aB3xYz"
        };
        let (status, _, _) = fixture.get(path, &[]).await;
        assert_ne!(status, StatusCode::TOO_MANY_REQUESTS, "第 {index} 次");
    }
    let (status, headers, html) = fixture
        .get("/gh/octo/hello", &[("accept-language", "zh-CN")])
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(headers[header::RETRY_AFTER], "60");
    assert!(html.contains("<h1>这一会儿打开得太多了</h1>"));
    assert!(!html.contains("octo"));
    assert_eq!(
        fixture.repositories.asked().len(),
        usize::try_from(PER_MINUTE / 2).unwrap(),
        "超了的那次不去查"
    );

    let (status, _, _) = fixture.get("/gh/-octo/hello", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, svg) = fixture.get("/gh/octo/hello/badge.svg", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(svg.contains("no room"));
    fixture.clock.advance(60_000);
    let (status, _, _) = fixture.get("/gh/octo/hello", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "过了一分钟又能打开");
}

#[tokio::test]
async fn 徽章写现在几个在线_缓存一分钟_带_etag_认_if_none_match() {
    let fixture = Fixture::new(FakeRepositories::answering(Ok(online_room("Book club", 7))));
    let (status, headers, svg) = fixture.get("/gh/octo/hello/badge.svg", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "image/svg+xml; charset=utf-8"
    );
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=300");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
    assert!(svg.contains("height=\"20\""));
    assert!(svg.contains(">Agent Room</text>"));
    assert!(svg.contains(">7 online</text>"));
    assert!(svg.contains("aria-label=\"Agent Room: 7 online\""));
    assert!(!svg.contains("octo") && !svg.contains("hello") && !svg.contains("Book club"));
    assert!(!svg.contains("style="), "不用内联样式");
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();
    assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");

    for matching in [
        etag.clone(),
        format!("W/{etag}"),
        format!("\"x\", {etag}"),
        "*".into(),
    ] {
        let (status, headers, body) = fixture
            .get(
                "/gh/octo/hello/badge.svg",
                &[("if-none-match", matching.as_str())],
            )
            .await;
        assert_eq!(status, StatusCode::NOT_MODIFIED, "{matching}");
        assert_eq!(headers[header::ETAG], etag.as_str());
        assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=300");
        assert!(body.is_empty());
    }
    let (status, _, _) = fixture
        .get(
            "/gh/octo/hello/badge.svg",
            &[("if-none-match", "\"other\"")],
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // 中文的另一张图，`ETag` 也不一样；同一个仓库一分钟内不再算。
    let (_, headers, svg) = fixture
        .get("/gh/OCTO/hello/badge.svg?lang=zh-CN", &[])
        .await;
    assert!(svg.contains(">7 在线</text>"));
    assert_ne!(headers[header::ETAG], etag.as_str());
    assert_eq!(fixture.repositories.asked().len(), 1);

    *fixture.repositories.answer.lock().unwrap() = Ok(online_room("Book club", 9));
    fixture.clock.advance(COUNT_TTL_MILLIS - 1);
    let (_, _, svg) = fixture.get("/gh/octo/hello/badge.svg", &[]).await;
    assert!(svg.contains(">7 online</text>"));
    fixture.clock.advance(1);
    let (_, headers, svg) = fixture.get("/gh/octo/hello/badge.svg", &[]).await;
    assert!(svg.contains(">9 online</text>"));
    assert_ne!(headers[header::ETAG], etag.as_str());
    assert_eq!(fixture.repositories.asked().len(), 2);

    *fixture.repositories.answer.lock().unwrap() = Ok({
        let mut room = online_room("Book club", 200);
        room.online = Some(PublicRoomOnline {
            count: 200,
            at_least: true,
        });
        room
    });
    fixture.clock.advance(COUNT_TTL_MILLIS);
    let (_, _, svg) = fixture.get("/gh/octo/hello/badge.svg", &[]).await;
    assert!(svg.contains(">200+ online</text>"));
}

#[tokio::test]
async fn 没有房间的徽章写no_room_记五分钟() {
    let fixture = Fixture::new(FakeRepositories::answering(Err(
        PublicDirectoryFailureKind::NotFound,
    )));
    let (status, headers, svg) = fixture.get("/gh/octo/hello/badge.svg", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=300");
    assert!(headers.contains_key(header::ETAG));
    assert!(svg.contains(">no room</text>"));
    let (_, _, svg) = fixture
        .get("/gh/octo/hello/badge.svg?lang=zh-CN", &[])
        .await;
    assert!(svg.contains(">没有房间</text>"));

    *fixture.repositories.answer.lock().unwrap() = Ok(online_room("Book club", 2));
    fixture.clock.advance(NO_ROOM_TTL_MILLIS - 1);
    let (_, _, svg) = fixture.get("/gh/octo/hello/badge.svg", &[]).await;
    assert!(svg.contains(">no room</text>"), "五分钟里不再查库");
    assert_eq!(fixture.repositories.asked().len(), 1);
    fixture.clock.advance(1);
    let (_, _, svg) = fixture.get("/gh/octo/hello/badge.svg", &[]).await;
    assert!(svg.contains(">2 online</text>"));

    // 绑着、在线数问不到：只有左边那段。
    *fixture.repositories.answer.lock().unwrap() = Ok({
        let mut room = online_room("Book club", 0);
        room.online = None;
        room
    });
    let (status, _, svg) = fixture.get("/gh/other/repo/badge.svg", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(svg.contains("aria-label=\"Agent Room\""));
    assert_eq!(svg.matches("</text>").count(), 1);
}

#[tokio::test]
async fn 每秒最多算五十个新的_超了给不带数的_库读不了也不记() {
    let fixture = Fixture::new(FakeRepositories::answering(Ok(online_room("Book club", 4))));
    for index in 0..NEW_PER_SECOND {
        let (_, headers, svg) = fixture
            .get(&format!("/gh/octo/repo-{index}/badge.svg"), &[])
            .await;
        assert!(svg.contains(">4 online</text>"), "第 {index} 个");
        assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=300");
    }
    let (status, headers, svg) = fixture.get("/gh/octo/one-more/badge.svg", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert!(!headers.contains_key(header::ETAG));
    assert!(svg.contains("aria-label=\"Agent Room\""));
    assert!(!svg.contains("online"));
    assert_eq!(
        fixture.repositories.asked().len(),
        usize::try_from(NEW_PER_SECOND).unwrap()
    );
    // 算过的照旧从内存里给。
    let (_, _, svg) = fixture.get("/gh/octo/repo-0/badge.svg", &[]).await;
    assert!(svg.contains(">4 online</text>"));

    fixture.clock.advance(1_000);
    let (_, _, svg) = fixture.get("/gh/octo/one-more/badge.svg", &[]).await;
    assert!(svg.contains(">4 online</text>"), "下一秒又能算");

    *fixture.repositories.answer.lock().unwrap() =
        Err(PublicDirectoryFailureKind::DependencyUnavailable);
    let asked = fixture.repositories.asked().len();
    for _ in 0..2 {
        let (status, headers, svg) = fixture.get("/gh/octo/down/badge.svg", &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert!(!svg.contains("online") && !svg.contains("no room"));
    }
    assert_eq!(
        fixture.repositories.asked().len(),
        asked + 2,
        "读不了的不记"
    );
}

#[tokio::test]
async fn 同一个仓库同时只算一次_别的请求等它() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let repositories = Arc::new(FakeRepositories {
        answer: Mutex::new(Ok(online_room("Book club", 5))),
        asked: Mutex::new(Vec::new()),
        gate: Some(gate.clone()),
        bound: Mutex::new(Ok(None)),
    });
    let fixture = Arc::new(Fixture::new(repositories));
    let requests: Vec<_> = (0..5)
        .map(|index| {
            let fixture = fixture.clone();
            let path = if index % 2 == 0 {
                "/gh/octo/hello/badge.svg"
            } else {
                "/gh/Octo/Hello/badge.svg?lang=zh-CN"
            };
            tokio::spawn(async move { fixture.get(path, &[]).await })
        })
        .collect();
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(fixture.repositories.asked().len(), 1, "只算一次");
    gate.add_permits(10);
    for request in requests {
        let (status, _, svg) = request.await.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert!(svg.contains(">5 online</text>") || svg.contains(">5 在线</text>"));
    }
    assert_eq!(fixture.repositories.asked().len(), 1);
}

#[tokio::test]
async fn 等着的请求走了也照样算完_记下结果() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let repositories = Arc::new(FakeRepositories {
        answer: Mutex::new(Ok(online_room("Book club", 6))),
        asked: Mutex::new(Vec::new()),
        gate: Some(gate.clone()),
        bound: Mutex::new(Ok(None)),
    });
    let fixture = Arc::new(Fixture::new(repositories));
    let waiting = {
        let fixture = fixture.clone();
        tokio::spawn(async move { fixture.get("/gh/octo/hello/badge.svg", &[]).await })
    };
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    waiting.abort();
    gate.add_permits(1);
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    let (_, _, svg) = fixture.get("/gh/octo/hello/badge.svg", &[]).await;
    assert!(svg.contains(">6 online</text>"));
    assert_eq!(fixture.repositories.asked().len(), 1, "没有再算一次");
}
