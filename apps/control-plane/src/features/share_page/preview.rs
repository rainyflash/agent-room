//! 每间房、每段对话自己的预览图（`specs/redesign/project-rooms.md` 的“预览图”，3P-8a）：
//! `GET /r/{分享码}/preview.png` 和 `GET /r/{分享码}/s/{段码}/preview.png`，1280×640 的 PNG。分享页、
//! 绑了仓库的 `/gh/` 页、一段对话那一页的 `og:image` 指着它们。
//!
//! - 只画此刻公开、没关、没被平台拿掉的房间（和分享页同一个 `detail`）；别的一律回分享页“没有这个码”那一页，
//!   一字不差。一段对话照那一页的规矩：没有、不放实况、加了密是同一个 404，看不了了是 410。
//! - 图上只有分享页上本来公开的：房间名、绑着的仓库、现在几个在线、几人几个 Agent（一段对话写几句），
//!   不画房间里说的话。中英文照房间名里有没有中文字（`guess_language`），爬虫多半不带 `Accept-Language`。
//! - 地址带 `?v=<摘要>`（房间名和几个数），这些变了地址就变，社交媒体会重新抓；服务器不看 `v`，照此刻的画。
//! - 同一张图在内存里留 10 分钟、最多 500 张；回答 `Cache-Control: public, max-age=600`，带 `ETag`。
//! - 画图吃 CPU：同时最多画 2 张，排 2 秒排不上就算忙；和分享页合起来按来源限流。忙不过来、画出错、
//!   字体读不了，都 302 到网页上的通用图（`room-share-preview-<语言>.png`），不缓存。

use std::{
    collections::HashMap,
    fmt::Write as _,
    hash::{DefaultHasher, Hash as _, Hasher as _},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, PoisonError},
    time::Duration,
};

use agent_room_application::{
    ports::Clock,
    public_room_directory::{PublicDirectoryFailureKind, PublicRoomDetailView},
};
use agent_room_domain::{
    ids::RoomCatalogId, project_rooms::GithubRepository, room_excerpts::RoomExcerptCode,
    room_listing::RoomShareCode,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use sha2::{Digest as _, Sha256};
use tokio::sync::Semaphore;

mod card;
mod draw;

use card::{CardKind, PreviewCard};
use draw::PreviewAssets;

use super::{
    Language, SharePageHttpState, accepted_language, badge::matches_if_none_match, excerpt_url,
    gone_excerpt_response, guess_language, not_found_page, render, share_url,
};
use crate::public_watch::{ExcerptPage, ExcerptPageFailure, ShownExcerpt};

/// 镜像里放字体的位置（`infra/production/Containerfile.control-plane`）。本地可以用
/// `AGENT_ROOM_PREVIEW_FONT` 指到仓库里的 `apps/control-plane/assets/fonts/NotoSansSC-Bold.otf`。
pub(crate) const DEFAULT_PREVIEW_FONT: &str = "/usr/share/agent-room/fonts/NotoSansSC-Bold.otf";
const IMAGE_CACHE: &str = "public, max-age=600";
/// 一张图在内存里留多久。
const KEEP_MILLIS: i64 = 10 * 60_000;
/// 最多留几张。满了先扔过期的，还满就扔最早画的。
const MAX_KEPT: usize = 500;
/// 同时最多画几张。
const MAX_DRAWING: usize = 2;
/// 排多久排不上就算忙。
const DRAW_WAIT: Duration = Duration::from_secs(2);
/// 图的样子改了就加一：地址里的 `v` 和 `ETag` 都跟着变，社交媒体会重新抓。
const LAYOUT_VERSION: &str = "1";

/// 画好的图、同时画几张、字体。只在这个进程的内存里，开几个控制面各画各的。
pub(crate) struct PreviewImages {
    font_path: PathBuf,
    /// 头一次画图时读字体；读不了是 `None`，只记一次告警，以后都给通用图。
    assets: OnceLock<Option<Arc<PreviewAssets>>>,
    kept: Mutex<HashMap<u64, Kept>>,
    drawing: Arc<Semaphore>,
    clock: Arc<dyn Clock>,
}

struct Kept {
    at: i64,
    png: Bytes,
}

impl PreviewImages {
    pub(crate) fn new(font_path: PathBuf, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self {
            font_path,
            assets: OnceLock::new(),
            kept: Mutex::new(HashMap::new()),
            drawing: Arc::new(Semaphore::new(MAX_DRAWING)),
            clock,
        })
    }

    /// 这张图的 PNG；忙、画不了是 `None`。
    async fn png(self: &Arc<Self>, card: PreviewCard, key: u64) -> Option<Bytes> {
        if let Some(png) = self.kept(key) {
            return Some(png);
        }
        if matches!(self.assets.get(), Some(None)) {
            return None;
        }
        let Ok(Ok(permit)) =
            tokio::time::timeout(DRAW_WAIT, Arc::clone(&self.drawing).acquire_owned()).await
        else {
            return None;
        };
        // 排队的时候别的请求可能已经画好了。
        if let Some(png) = self.kept(key) {
            return Some(png);
        }
        let images = Arc::clone(self);
        // 名额跟着画图的任务走：等着的请求走了，那一张也照样画完、记下，同时画的不会超过两张。
        let drawn = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            match images.assets()?.draw(&card) {
                Ok(png) => {
                    let png = Bytes::from(png);
                    images.keep(key, png.clone());
                    Some(png)
                }
                Err(error) => {
                    tracing::warn!(%error, "预览图没画成");
                    None
                }
            }
        })
        .await;
        drawn.unwrap_or_else(|error| {
            tracing::warn!(%error, "预览图没画完");
            None
        })
    }

    /// 画图要的字体和机器人，一个进程只读一次。在画图的线程里调：读文件、解字体要一会儿。
    fn assets(&self) -> Option<Arc<PreviewAssets>> {
        self.assets
            .get_or_init(|| match PreviewAssets::load(&self.font_path) {
                Ok(assets) => Some(Arc::new(assets)),
                Err(error) => {
                    tracing::warn!(
                        path = %self.font_path.display(),
                        %error,
                        "预览图的字体读不了，分享出去的预览一律用通用图"
                    );
                    None
                }
            })
            .clone()
    }

    fn kept(&self, key: u64) -> Option<Bytes> {
        let now = self.clock.now().value();
        let kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        kept.get(&key)
            .filter(|kept| fresh(kept.at, now))
            .map(|kept| kept.png.clone())
    }

    fn keep(&self, key: u64, png: Bytes) {
        let now = self.clock.now().value();
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        if !kept.contains_key(&key) && kept.len() >= MAX_KEPT {
            kept.retain(|_, kept| fresh(kept.at, now));
            if kept.len() >= MAX_KEPT
                && let Some(oldest) = kept
                    .iter()
                    .min_by_key(|(_, kept)| kept.at)
                    .map(|(key, _)| *key)
            {
                kept.remove(&oldest);
            }
        }
        kept.insert(key, Kept { at: now, png });
    }
}

fn fresh(at: i64, now: i64) -> bool {
    (at..at + KEEP_MILLIS).contains(&now)
}

/// `GET /r/{分享码}/preview.png`。
pub(super) async fn room_image(
    State(state): State<SharePageHttpState>,
    Path(share_code): Path<String>,
    headers: HeaderMap,
) -> Response {
    let accepted = accepted_language(&headers);
    if state.limits.admit(&headers).is_err() {
        return fallback(&state, accepted.unwrap_or(Language::English));
    }
    let detail = match state.directory.detail(None, share_code).await {
        Ok(detail) => detail,
        Err(failure) => {
            return match failure.kind() {
                PublicDirectoryFailureKind::NotFound
                | PublicDirectoryFailureKind::InvalidRequest => not_found_page(&state, accepted),
                PublicDirectoryFailureKind::DependencyUnavailable
                | PublicDirectoryFailureKind::Internal => {
                    tracing::info!(
                        operation = failure.operation(),
                        failure = ?failure.kind(),
                        "预览图没读到房间"
                    );
                    fallback(&state, accepted.unwrap_or(Language::English))
                }
            };
        }
    };
    let room = &detail.room.room;
    let Some(repository) = bound_repository(&state, room.catalog_id).await else {
        return fallback(&state, guess_language(&room.name));
    };
    let card = room_card(&state, &detail, repository.as_ref());
    respond(&state, &headers, card).await
}

/// 一间房的图上画什么。
fn room_card(
    state: &SharePageHttpState,
    detail: &PublicRoomDetailView,
    repository: Option<&GithubRepository>,
) -> PreviewCard {
    let room = &detail.room.room;
    PreviewCard {
        language: guess_language(&room.name),
        name: room.name.clone(),
        address: address(state, room.share_code.as_str(), repository),
        repository: repository.map(GithubRepository::full_name),
        kind: CardKind::Room {
            online: detail.online.map(|online| (online.count, online.at_least)),
            people: room.member_count,
            agents: room.agent_count,
        },
    }
}

/// `GET /r/{分享码}/s/{段码}/preview.png`：和那一段的页面同样的规矩。
pub(super) async fn excerpt_image(
    State(state): State<SharePageHttpState>,
    Path((share_code, code)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let accepted = accepted_language(&headers);
    if state.limits.admit(&headers).is_err() {
        return fallback(&state, accepted.unwrap_or(Language::English));
    }
    let (Ok(share_code), Ok(code)) = (
        RoomShareCode::parse(&share_code),
        RoomExcerptCode::parse(&code),
    ) else {
        return not_found_page(&state, accepted);
    };
    let page = match state.excerpts.page(&share_code, &code).await {
        Ok(page) => page,
        Err(ExcerptPageFailure::NotFound) => return not_found_page(&state, accepted),
        Err(ExcerptPageFailure::Unavailable) => {
            return fallback(&state, accepted.unwrap_or(Language::English));
        }
    };
    let excerpt = match page.as_ref() {
        ExcerptPage::Shown(excerpt) => excerpt,
        ExcerptPage::Gone { room_name } => {
            return gone_excerpt_response(&state, &share_code, room_name, accepted);
        }
    };
    let language = guess_language(&excerpt.room_name);
    // 绑着的仓库按房间找：先拿分享码问一次目录（和分享页同一个），房间此刻不公开了就和没有这一段一样。
    let catalog_id = match state
        .directory
        .detail(None, share_code.as_str().to_owned())
        .await
    {
        Ok(detail) => detail.room.room.catalog_id,
        Err(failure) => {
            return match failure.kind() {
                PublicDirectoryFailureKind::NotFound
                | PublicDirectoryFailureKind::InvalidRequest => not_found_page(&state, accepted),
                PublicDirectoryFailureKind::DependencyUnavailable
                | PublicDirectoryFailureKind::Internal => fallback(&state, language),
            };
        }
    };
    let Some(repository) = bound_repository(&state, catalog_id).await else {
        return fallback(&state, language);
    };
    let card = PreviewCard {
        language,
        name: excerpt.room_name.clone(),
        address: address(&state, share_code.as_str(), repository.as_ref()),
        repository: repository.as_ref().map(GithubRepository::full_name),
        kind: CardKind::Excerpt {
            messages: message_count(excerpt),
        },
    };
    respond(&state, &headers, card).await
}

/// 这间房绑着的仓库。外面那层 `None` 是没读到。
async fn bound_repository(
    state: &SharePageHttpState,
    catalog_id: RoomCatalogId,
) -> Option<Option<GithubRepository>> {
    match state.repositories.bound_repository(catalog_id).await {
        Ok(repository) => Some(repository),
        Err(failure) => {
            tracing::info!(
                operation = failure.operation(),
                failure = ?failure.kind(),
                "预览图没读到绑着的仓库"
            );
            None
        }
    }
}

/// 左下写的地址：绑了仓库的写 `/gh/owner/repo`，别的写分享链接。
fn address(
    state: &SharePageHttpState,
    share_code: &str,
    repository: Option<&GithubRepository>,
) -> String {
    let site = &*state.site_origin;
    let host = site.strip_prefix("https://").unwrap_or(site);
    repository.map_or_else(
        || format!("{host}/r/{share_code}"),
        |repository| format!("{host}/gh/{}", repository.full_name()),
    )
}

fn message_count(excerpt: &ShownExcerpt) -> u32 {
    u32::try_from(excerpt.lines.len()).unwrap_or(u32::MAX)
}

async fn respond(state: &SharePageHttpState, request: &HeaderMap, card: PreviewCard) -> Response {
    let key = card_key(&card);
    let etag = format!("\"{key:016x}\"");
    if matches_if_none_match(request, &etag) {
        return image_headers(StatusCode::NOT_MODIFIED.into_response(), &etag);
    }
    let language = card.language;
    match state.previews.png(card, key).await {
        Some(png) => {
            let mut response = (StatusCode::OK, png).into_response();
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
            image_headers(response, &etag)
        }
        None => fallback(state, language),
    }
}

fn image_headers(mut response: Response, etag: &str) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(IMAGE_CACHE));
    if let Ok(etag) = HeaderValue::from_str(etag) {
        headers.insert(header::ETAG, etag);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("x-robots-tag"),
        HeaderValue::from_static("noindex"),
    );
    response
}

/// 忙不过来、画不了：跳到网页上的通用图，不缓存。
fn fallback(state: &SharePageHttpState, language: Language) -> Response {
    let location = render::generic_preview(&state.app_origin, language);
    let mut response = StatusCode::FOUND.into_response();
    let headers = response.headers_mut();
    if let Ok(location) = HeaderValue::from_str(&location) {
        headers.insert(header::LOCATION, location);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        HeaderName::from_static("x-robots-tag"),
        HeaderValue::from_static("noindex"),
    );
    response
}

/// 内存里认图、`ETag` 用的：图上画的全部东西，加上样子的版本。
fn card_key(card: &PreviewCard) -> u64 {
    let mut hasher = DefaultHasher::new();
    LAYOUT_VERSION.hash(&mut hasher);
    card.hash(&mut hasher);
    hasher.finish()
}

/// 分享页（和绑着仓库的 `/gh/` 页）的 `og:image`：`v` 是房间名、在线数、人数和 Agent 数的摘要。
/// 绑没绑仓库不算进去：要多查一次库，`/gh/` 页还得和 `/r/` 一字不差；绑上、解绑以后社交媒体那边要等它
/// 自己过期。
pub(super) fn room_image_url(site: &str, detail: &PublicRoomDetailView) -> String {
    let room = &detail.room.room;
    let online = detail.online.map_or_else(String::new, |online| {
        format!("{}{}", online.count, if online.at_least { "+" } else { "" })
    });
    let version = version(&[
        &room.name,
        &online,
        &room.member_count.to_string(),
        &room.agent_count.to_string(),
    ]);
    format!(
        "{}/preview.png?v={version}",
        share_url(site, room.share_code.as_str())
    )
}

/// 一段对话那一页的 `og:image`：`v` 是房间名和几句的摘要。
pub(super) fn excerpt_image_url(
    site: &str,
    share_code: &str,
    excerpt_code: &str,
    excerpt: &ShownExcerpt,
) -> String {
    let version = version(&[&excerpt.room_name, &message_count(excerpt).to_string()]);
    format!(
        "{}/preview.png?v={version}",
        excerpt_url(site, share_code, excerpt_code)
    )
}

/// 摘要的前 12 个十六进制字。
fn version(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(LAYOUT_VERSION.as_bytes());
    for part in parts {
        hasher.update([0]);
        hasher.update(part.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .take(6)
        .fold(String::with_capacity(12), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

#[cfg(test)]
mod tests;
