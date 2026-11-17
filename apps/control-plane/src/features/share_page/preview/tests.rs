//! 预览图（3P-8a）：真字体画出来的样子、`/r/{分享码}/preview.png` 的回答、缓存、忙和画不了时跳到通用图、
//! 分享页和 `/gh/` 页的 `og:image`。一段对话的图在 `excerpt_tests.rs`。

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use agent_room_application::public_room_directory::{PublicDirectoryFailureKind, PublicRoomOnline};
use agent_room_domain::{project_rooms::GithubRepository, room_listing::RoomJoinMode};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use resvg::tiny_skia::Pixmap;
use tower::ServiceExt;
use url::Url;

use super::{
    MAX_DRAWING, PreviewImages,
    card::{CardKind, Measure as _, PreviewCard, ROBOTS, wrap},
    card_key,
    draw::PreviewAssets,
    room_image_url,
};
use crate::features::{
    anonymous_limits::{AnonymousLimiter, PER_MINUTE},
    share_page::{
        Language, PAGE_POLICY, SharePageHttpState, router,
        tests::{FakeDirectory, FakeLive, FakeRepositories, TestClock, detail},
    },
};

fn font_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/NotoSansSC-Bold.otf")
}

fn missing_font() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/not-there.otf")
}

struct Fixture {
    app: Router,
    directory: Arc<FakeDirectory>,
    repositories: Arc<FakeRepositories>,
    state: SharePageHttpState,
}

impl Fixture {
    fn new(name: &str, font: PathBuf) -> Self {
        let directory = FakeDirectory::new(detail(name, "", RoomJoinMode::Open, false));
        let repositories = FakeRepositories::answering(Err(PublicDirectoryFailureKind::NotFound));
        let state = SharePageHttpState::new(
            directory.clone(),
            repositories.clone(),
            Arc::new(FakeLive::default()),
            Arc::new(TestClock),
            Arc::new(AnonymousLimiter::new(Vec::new(), Arc::new(TestClock))),
            "agent-room.test",
            &Url::parse("https://app.agent-room.test").unwrap(),
        )
        .with_preview_font(font);
        Self {
            app: router(state.clone()),
            directory,
            repositories,
            state,
        }
    }

    async fn get(&self, path: &str, headers: &[(&str, &str)]) -> (StatusCode, HeaderMap, Vec<u8>) {
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
        let body = to_bytes(response.into_body(), 4 * 1_024 * 1_024)
            .await
            .unwrap();
        (status, headers, body.to_vec())
    }
}

fn card(name: &str, language: Language) -> PreviewCard {
    PreviewCard {
        language,
        name: name.to_owned(),
        repository: None,
        address: "agent-room.test/r/aB3xYz".to_owned(),
        kind: CardKind::Room {
            online: Some((12, false)),
            people: 3,
            agents: 5,
        },
    }
}

/// PNG 解开以后某一点的颜色（去掉预乘）。
fn pixel(png: &[u8], x: u32, y: u32) -> (u8, u8, u8) {
    let image = Pixmap::decode_png(png).unwrap();
    let color = image.pixel(x, y).unwrap().demultiply();
    (color.red(), color.green(), color.blue())
}

/// 一块地方里有几个点照 `test` 算。
fn count_pixels(png: &[u8], area: (u32, u32, u32, u32), test: fn(u8, u8, u8) -> bool) -> usize {
    let image = Pixmap::decode_png(png).unwrap();
    let (left, top, right, bottom) = area;
    (top..bottom)
        .flat_map(|y| (left..right).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let color = image.pixel(*x, *y).unwrap().demultiply();
            test(color.red(), color.green(), color.blue())
        })
        .count()
}

fn dark(red: u8, green: u8, blue: u8) -> bool {
    red < 80 && green < 80 && blue < 80
}

/// 机器人身上的青色：绿和蓝都比红高出一截。
fn teal(red: u8, green: u8, blue: u8) -> bool {
    green > 120 && green.saturating_sub(red) > 60 && blue.saturating_sub(red) > 40
}

#[test]
fn 真字体量得出中英文的字宽_长名字两行省略() {
    let assets = PreviewAssets::load(&font_path()).expect("仓库里带着字体");
    let metrics = assets.metrics().unwrap();
    // 中文字一个 em，英文字母比它窄，表情没有。
    assert!((metrics.advance('读').unwrap() - 1.0).abs() < 0.01);
    let latin = metrics.advance('a').unwrap();
    assert!(latin > 0.3 && latin < 0.8, "{latin}");
    assert!(metrics.advance('…').is_some());
    assert_eq!(metrics.advance('🚀'), None);

    let long = "这是一个名字特别特别长的房间用来看看预览图上的房间名会不会好好地换成两行再省略";
    let lines = wrap(long, 56.0, 700.0, 2, &metrics);
    assert_eq!(lines.len(), 2);
    assert!(lines[1].text.ends_with('…'), "{lines:?}");
    for line in &lines {
        assert!(super::card::width(&line.text, 56.0, &metrics) <= 700.0);
    }
    let latin = "A weekly reading circle for people who love very long and winding books";
    let lines = wrap(latin, 56.0, 700.0, 2, &metrics);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].text.starts_with("A weekly reading"), "{lines:?}");
    assert!(lines[1].text.ends_with('…'), "{lines:?}");
}

#[test]
fn 画出来是_1280x640_的_png_白底_有字_机器人有颜色() {
    let assets = PreviewAssets::load(&font_path()).unwrap();
    let mut card = card("rainyflash/agent-room", Language::Chinese);
    card.repository = Some("rainyflash/agent-room".to_owned());
    let png = assets.draw(&card).unwrap();

    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let image = Pixmap::decode_png(&png).unwrap();
    assert_eq!((image.width(), image.height()), (1_280, 640));
    // 四个角和中间的空白是白的。
    assert_eq!(pixel(&png, 4, 4), (255, 255, 255));
    assert_eq!(pixel(&png, 1_276, 636), (255, 255, 255));
    assert_eq!(pixel(&png, 760, 140), (255, 255, 255));
    // 左上的“Agent Room”、房间名那一块、下面那一行都有黑字。
    assert!(count_pixels(&png, (80, 70, 240, 104), dark) > 50);
    assert!(count_pixels(&png, (80, 190, 780, 420), dark) > 500);
    // 颜色只在机器人那边：左边写字的地方一个青色的点也没有。
    assert_eq!(count_pixels(&png, (0, 0, 760, 640), teal), 0);
    // 机器人底下的圆：圆里靠边的地方是圆的颜色（机器人贴在中间）。
    let square = &ROBOTS[0];
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let edge = (
        square.center.0 as u32,
        (square.center.1 - square.radius + 6.0) as u32,
    );
    assert_eq!(pixel(&png, edge.0, edge.1), (0xe2, 0xf3, 0xef));
    // 机器人的脸（白）和眼睛（深色）在圆心附近。
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let center = (square.center.0 as u32, square.center.1 as u32);
    let around = (center.0 - 60, center.1 - 60, center.0 + 60, center.1 + 60);
    assert!(count_pixels(&png, around, dark) > 20, "轮廓和眼睛");
    assert!(count_pixels(&png, around, teal) > 500, "青色的身子");
}

#[tokio::test]
async fn 公开着的房间出图_缓存十分钟_带_etag_认_if_none_match() {
    let fixture = Fixture::new("读书会", font_path());
    let (status, headers, png) = fixture.get("/r/aB3xYz/preview.png?v=abc", &[]).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=600");
    assert_eq!(headers["x-robots-tag"], "noindex");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(headers[header::CONTENT_SECURITY_POLICY], PAGE_POLICY);
    assert!(png.starts_with(b"\x89PNG"));
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();

    // 内存里留着：两个画图的名额都占着，照样马上回同一张。
    let _busy = fixture
        .state
        .previews
        .drawing
        .clone()
        .acquire_many_owned(u32::try_from(MAX_DRAWING).unwrap())
        .await
        .unwrap();
    let (status, again, cached) = fixture.get("/r/aB3xYz/preview.png", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cached, png);
    assert_eq!(again[header::ETAG], etag.as_str());

    let (status, headers, body) = fixture
        .get("/r/aB3xYz/preview.png", &[("if-none-match", &etag)])
        .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=600");
    assert_eq!(headers[header::ETAG], etag.as_str());
    assert_eq!(
        fixture.directory.asked.lock().unwrap().len(),
        3,
        "每次都重查房间"
    );
}

#[tokio::test]
async fn 不公开的房间回和分享页一模一样的_404() {
    let fixture = Fixture::new("读书会", font_path());
    let (page_status, page_headers, page) = fixture
        .get("/r/Zz9Zz9", &[("accept-language", "zh-CN")])
        .await;
    assert_eq!(page_status, StatusCode::NOT_FOUND);

    for path in [
        "/r/Zz9Zz9/preview.png",
        "/r/not-a-code/preview.png",
        "/r/%3Cscript%3E/preview.png",
    ] {
        let (status, headers, body) = fixture.get(path, &[("accept-language", "zh-CN")]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(body, page, "{path}");
        for name in [
            header::CACHE_CONTROL,
            header::CONTENT_TYPE,
            header::CONTENT_SECURITY_POLICY,
            header::VARY,
        ] {
            assert_eq!(headers[&name], page_headers[&name], "{path} {name}");
        }
        assert_eq!(headers["x-robots-tag"], "noindex");
    }
}

#[tokio::test]
async fn 字体读不了_库读不了都跳到通用图_不缓存() {
    let fixture = Fixture::new("读书会", missing_font());
    for _ in 0..2 {
        let (status, headers, _) = fixture.get("/r/aB3xYz/preview.png", &[]).await;
        assert_eq!(status, StatusCode::FOUND);
        assert_eq!(
            headers[header::LOCATION],
            "https://app.agent-room.test/room-share-preview-zh-CN.png"
        );
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert!(headers.get(header::ETAG).is_none());
    }
    assert!(
        matches!(fixture.state.previews.assets.get(), Some(None)),
        "只读一次"
    );

    let english = Fixture::new("Book club", missing_font());
    let (_, headers, _) = english.get("/r/aB3xYz/preview.png", &[]).await;
    assert_eq!(
        headers[header::LOCATION],
        "https://app.agent-room.test/room-share-preview-en.png"
    );

    let fixture = Fixture::new("读书会", font_path());
    *fixture.directory.failure.lock().unwrap() =
        Some(PublicDirectoryFailureKind::DependencyUnavailable);
    let (status, headers, _) = fixture.get("/r/aB3xYz/preview.png", &[]).await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");

    let fixture = Fixture::new("读书会", font_path());
    *fixture.repositories.bound.lock().unwrap() =
        Err(PublicDirectoryFailureKind::DependencyUnavailable);
    let (status, headers, _) = fixture.get("/r/aB3xYz/preview.png", &[]).await;
    assert_eq!(status, StatusCode::FOUND, "绑着哪个仓库读不到也不画");
    assert_eq!(
        headers[header::LOCATION],
        "https://app.agent-room.test/room-share-preview-zh-CN.png"
    );
}

#[tokio::test(start_paused = true)]
async fn 同时画的满了_排不上就跳到通用图() {
    let fixture = Fixture::new("读书会", font_path());
    let _busy = fixture
        .state
        .previews
        .drawing
        .clone()
        .acquire_many_owned(u32::try_from(MAX_DRAWING).unwrap())
        .await
        .unwrap();
    let (status, headers, _) = fixture.get("/r/aB3xYz/preview.png", &[]).await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(
        headers[header::LOCATION],
        "https://app.agent-room.test/room-share-preview-zh-CN.png"
    );
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn 和分享页合起来按来源限流_超了跳到通用图_不去查() {
    let fixture = Fixture::new("读书会", missing_font());
    for index in 0..PER_MINUTE {
        let (status, _, _) = fixture.get("/r/aB3xYz", &[]).await;
        assert_eq!(status, StatusCode::OK, "第 {index} 次");
    }
    let asked = fixture.directory.asked.lock().unwrap().len();
    let (status, headers, _) = fixture
        .get("/r/aB3xYz/preview.png", &[("accept-language", "en")])
        .await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(
        headers[header::LOCATION],
        "https://app.agent-room.test/room-share-preview-en.png"
    );
    assert_eq!(fixture.directory.asked.lock().unwrap().len(), asked);
}

#[tokio::test]
async fn 绑着仓库的房间图上写仓库_地址写_gh() {
    let fixture = Fixture::new("Book club", missing_font());
    *fixture.repositories.bound.lock().unwrap() =
        Ok(Some(GithubRepository::new("Octo", "Hello").unwrap()));
    let room = fixture.directory.room.lock().unwrap().clone();
    let bound = super::room_card(
        &fixture.state,
        &room,
        Some(&GithubRepository::new("Octo", "Hello").unwrap()),
    );
    assert_eq!(bound.repository.as_deref(), Some("Octo/Hello"));
    assert_eq!(bound.address, "agent-room.test/gh/Octo/Hello");
    assert_eq!(bound.language, Language::English);
    let plain = super::room_card(&fixture.state, &room, None);
    assert_eq!(plain.address, "agent-room.test/r/aB3xYz");
    assert_ne!(card_key(&bound), card_key(&plain), "绑上仓库是另一张图");
}

#[test]
fn 地址里的摘要跟着房间名和在线数变() {
    let site = "https://agent-room.test";
    let mut room = detail("读书会", "", RoomJoinMode::Open, false);
    let first = room_image_url(site, &room);
    assert!(
        first.starts_with("https://agent-room.test/r/aB3xYz/preview.png?v="),
        "{first}"
    );
    let digest = first.rsplit('=').next().unwrap();
    assert_eq!(digest.len(), 12);
    assert!(
        digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
    assert_eq!(room_image_url(site, &room), first, "一样的房间一样的地址");

    room.room.room.description = "简介不在图上".to_owned();
    assert_eq!(room_image_url(site, &room), first, "简介变了不换");

    let mut renamed = room.clone();
    renamed.room.room.name = "读书会二".to_owned();
    assert_ne!(room_image_url(site, &renamed), first);

    let mut online = room.clone();
    online.online = Some(PublicRoomOnline {
        count: 4,
        at_least: false,
    });
    let with_online = room_image_url(site, &online);
    assert_ne!(with_online, first);
    online.online = Some(PublicRoomOnline {
        count: 5,
        at_least: false,
    });
    assert_ne!(room_image_url(site, &online), with_online);
    online.online = Some(PublicRoomOnline {
        count: 5,
        at_least: true,
    });
    assert_ne!(room_image_url(site, &online), with_online);
}

#[tokio::test]
async fn 分享页和绑着的仓库地址都指着这间房自己的图() {
    let fixture = Fixture::new("读书会", missing_font());
    let room = fixture.directory.room.lock().unwrap().clone();
    let expected = format!(
        "<meta property=\"og:image\" content=\"{}\">",
        room_image_url("https://agent-room.test", &room)
    );
    for language in ["zh-CN", "en"] {
        let (_, _, html) = fixture
            .get("/r/aB3xYz", &[("accept-language", language)])
            .await;
        let html = String::from_utf8(html).unwrap();
        assert!(html.contains(&expected), "{language}: {html}");
        assert!(html.contains("<meta property=\"og:image:width\" content=\"1280\">"));
        assert!(!html.contains("room-share-preview"), "{language}");
    }

    *fixture.repositories.answer.lock().unwrap() = Ok(room);
    let (status, _, html) = fixture.get("/gh/octo/hello", &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(html).unwrap().contains(&expected));

    // 没有这间房的那一页照旧用通用图。
    let (_, _, html) = fixture.get("/r/Zz9Zz9", &[("accept-language", "en")]).await;
    assert!(String::from_utf8(html).unwrap().contains(
        "<meta property=\"og:image\" content=\"https://app.agent-room.test/room-share-preview-en.png\">"
    ));
}

#[test]
fn 字体只读一次_读不了也不会_panic() {
    let images = PreviewImages::new(missing_font(), Arc::new(TestClock));
    assert!(images.assets().is_none());
    assert!(images.assets().is_none());
}
