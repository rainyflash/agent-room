//! 公开房间的分享页（`specs/public-rooms/design.md` 的“分享链接”）：`GET /r/{分享码}` 出整页 HTML，
//! 不要登录，也不看登录。Caddy 在裸域上把 `/r/*` 转到这里。
//!
//! 裸域的内容安全策略只许同源的样式和脚本、不许内联，所以样式和“复制链接”的脚本是两个固定名字的
//! 文件 `/r/_/share.css`、`/r/_/share.js`。分享码只有字母和数字，碰不上 `_`。
//!
//! 公开着的房间缓存 60 秒，房间改名、改回私密以后一分钟内就变；不公开了、关了、码换了、没有这个码
//! 一律回同一个 404，不写房间名，也不缓存。都带 `X-Robots-Tag: noindex`：房间名和简介是用户写的，
//! 平台不审，不替它们进搜索结果。
//!
//! 页面和发现页的两个接口合起来按来源限流（[`AnonymousLimiter`]），样式和脚本不限。
//!
//! 项目房间（`specs/redesign/project-rooms.md` 的 3P-3）的仓库地址也在这里，Caddy 一样把 `/gh/*` 转过来：
//! - `GET /gh/{owner}/{repo}`：名字照 GitHub 的规矩先验，不合的直接 404、不查库。绑着这个仓库、此刻公开着
//!   的房间出它的分享页（和 `/r/<分享码>` 同一页）；别的一律出同一页“还没人认领”，回 404，不说是哪一种。
//!   不读 GitHub。和分享页一样按来源限流、`noindex`、缓存 60 秒，语言照同一个规则。
//! - `GET /gh/{owner}/{repo}/badge.svg`：README 徽章，见 [`badge`]。不按来源限流。

use std::sync::Arc;

use agent_room_application::{
    ports::Clock,
    project_rooms::RepositoryRoomPages,
    public_room_directory::{PublicDirectoryFailureKind, PublicRoomDirectoryUseCases},
};
use agent_room_domain::project_rooms::GithubRepository;
use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};

mod badge;
mod render;
mod unclaimed;

use badge::BadgeCounts;
use render::Origins;
// 提醒邮件的退订页也是一整页 HTML，和这里用同一套语言判断和转义。
pub(crate) use render::{Language, escape};

use crate::features::anonymous_limits::AnonymousLimiter;

const STYLESHEET: &str = include_str!("share_page/share.css");
const SCRIPT: &str = include_str!("share_page/share.js");
const ROOM_CACHE: &str = "public, max-age=60";
/// 样式和脚本的名字是固定的，发版以后最多五分钟就换成新的。
const ASSET_CACHE: &str = "public, max-age=300";

#[derive(Clone)]
pub(crate) struct SharePageHttpState {
    directory: Arc<dyn PublicRoomDirectoryUseCases>,
    clock: Arc<dyn Clock>,
    limits: Arc<AnonymousLimiter>,
    /// 仓库地址 `/gh/<owner>/<repo>` 认的房间。
    repositories: Arc<dyn RepositoryRoomPages>,
    badges: Arc<BadgeCounts>,
    /// 分享链接所在的网站，`https://<Matrix 服务器名>`。
    site_origin: Arc<str>,
    /// 网页应用，按钮去它的 `/explore/<分享码>`。
    app_origin: Arc<str>,
}

impl SharePageHttpState {
    pub(crate) fn new(
        directory: Arc<dyn PublicRoomDirectoryUseCases>,
        repositories: Arc<dyn RepositoryRoomPages>,
        clock: Arc<dyn Clock>,
        limits: Arc<AnonymousLimiter>,
        server_name: &str,
        frontend_origin: &url::Url,
    ) -> Self {
        Self {
            directory,
            badges: BadgeCounts::new(repositories.clone(), clock.clone()),
            repositories,
            clock,
            limits,
            site_origin: site_origin(server_name).into(),
            app_origin: frontend_origin.origin().ascii_serialization().into(),
        }
    }

    fn origins(&self) -> Origins<'_> {
        Origins {
            site: &self.site_origin,
            app: &self.app_origin,
        }
    }
}

pub(crate) fn router(state: SharePageHttpState) -> Router {
    Router::new()
        .route("/r/_/share.css", get(stylesheet))
        .route("/r/_/share.js", get(script))
        .route("/r/{share_code}", get(page))
        .route("/gh/{owner}/{repo}", get(repository_page))
        .route("/gh/{owner}/{repo}/badge.svg", get(badge::badge))
        .with_state(state)
}

async fn page(
    State(state): State<SharePageHttpState>,
    Path(share_code): Path<String>,
    headers: HeaderMap,
) -> Response {
    let accepted = accepted_language(&headers);
    if let Some(busy) = admit(&state, &headers, accepted) {
        return busy;
    }
    match state.directory.detail(None, share_code).await {
        Ok(detail) => {
            let language = accepted.unwrap_or_else(|| guess_language(&detail.room.room.name));
            let html = render::room_page(&detail, language, &state.origins(), state.clock.now());
            html_response(StatusCode::OK, ROOM_CACHE, html)
        }
        Err(failure) => {
            let language = accepted.unwrap_or(Language::English);
            match failure.kind() {
                PublicDirectoryFailureKind::NotFound
                | PublicDirectoryFailureKind::InvalidRequest => html_response(
                    StatusCode::NOT_FOUND,
                    "no-store",
                    render::gone_page(language, &state.origins()),
                ),
                PublicDirectoryFailureKind::DependencyUnavailable
                | PublicDirectoryFailureKind::Internal => {
                    tracing::info!(
                        operation = failure.operation(),
                        failure = ?failure.kind(),
                        "分享页没读到房间"
                    );
                    html_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "no-store",
                        render::unavailable_page(language, &state.origins()),
                    )
                }
            }
        }
    }
}

/// 仓库地址：绑着、公开着的房间出分享页，别的都是“还没人认领”。
async fn repository_page(
    State(state): State<SharePageHttpState>,
    Path((owner, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let accepted = accepted_language(&headers);
    let Ok(repository) = GithubRepository::new(&owner, &name) else {
        let language = accepted.unwrap_or(Language::English);
        return html_response(
            StatusCode::NOT_FOUND,
            "no-store",
            unclaimed::invalid_page(language, &state.origins()),
        );
    };
    if let Some(busy) = admit(&state, &headers, accepted) {
        return busy;
    }
    match state.repositories.public_room(repository.clone()).await {
        Ok(detail) => {
            let language = accepted.unwrap_or_else(|| guess_language(&detail.room.room.name));
            let html = render::room_page(&detail, language, &state.origins(), state.clock.now());
            html_response(StatusCode::OK, ROOM_CACHE, html)
        }
        Err(failure) => {
            let language = accepted.unwrap_or(Language::English);
            match failure.kind() {
                // 哪一种“没有”都是同一页，谁看都一样，照样缓存一分钟。
                PublicDirectoryFailureKind::NotFound
                | PublicDirectoryFailureKind::InvalidRequest => html_response(
                    StatusCode::NOT_FOUND,
                    ROOM_CACHE,
                    unclaimed::unclaimed_page(&repository, language, &state.origins()),
                ),
                PublicDirectoryFailureKind::DependencyUnavailable
                | PublicDirectoryFailureKind::Internal => {
                    tracing::info!(
                        operation = failure.operation(),
                        failure = ?failure.kind(),
                        "仓库地址没读到房间"
                    );
                    html_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "no-store",
                        render::unavailable_page(language, &state.origins()),
                    )
                }
            }
        }
    }
}

/// 按来源限流：这个来源一分钟里打开得太多时回“等一分钟”那一页，不去查。
fn admit(
    state: &SharePageHttpState,
    headers: &HeaderMap,
    accepted: Option<Language>,
) -> Option<Response> {
    let limited = state.limits.admit(headers).err()?;
    let html = render::busy_page(accepted.unwrap_or(Language::English), &state.origins());
    let mut response = html_response(StatusCode::TOO_MANY_REQUESTS, "no-store", html);
    response.headers_mut().insert(
        header::RETRY_AFTER,
        HeaderValue::from(limited.retry_after_seconds),
    );
    Some(response)
}

async fn stylesheet() -> Response {
    asset("text/css; charset=utf-8", STYLESHEET)
}

async fn script() -> Response {
    asset("text/javascript; charset=utf-8", SCRIPT)
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, ASSET_CACHE),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body,
    )
        .into_response()
}

fn html_response(status: StatusCode, cache: &'static str, html: String) -> Response {
    let mut response = (status, html).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    // 缓存按浏览器的语言分开存，中文和英文的页面不混。
    headers.insert(header::VARY, HeaderValue::from_static("Accept-Language"));
    headers.insert(
        HeaderName::from_static("x-robots-tag"),
        HeaderValue::from_static("noindex"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// 分享链接所在的网站：`https://<Matrix 服务器名>`，也就是裸域。
pub(crate) fn site_origin(server_name: &str) -> String {
    format!("https://{server_name}")
}

/// 一间公开房间的分享链接。分享页、公开设置和发现页的回答都用它，网页不用自己猜裸域在哪。
pub(crate) fn share_url(site_origin: &str, share_code: &str) -> String {
    format!("{site_origin}/r/{share_code}")
}

/// 按 `Accept-Language` 的先后（`q` 大的在前）找第一个中文或英文。没有这个头、或者里面两种都没有
/// 时是 `None`。
pub(crate) fn accepted_language(headers: &HeaderMap) -> Option<Language> {
    let value = headers.get(header::ACCEPT_LANGUAGE)?.to_str().ok()?;
    let mut ranges: Vec<(u16, &str)> = value
        .split(',')
        .filter_map(|range| {
            let mut parts = range.split(';');
            let tag = parts.next()?.trim();
            let quality = parts
                .find_map(|part| part.trim().strip_prefix("q="))
                .map_or(Some(1_000), quality_per_mille)?;
            (quality > 0 && !tag.is_empty()).then_some((quality, tag))
        })
        .collect();
    // 稳定排序：一样的 `q` 照写的先后。
    ranges.sort_by_key(|(quality, _)| std::cmp::Reverse(*quality));
    ranges.into_iter().find_map(|(_, tag)| {
        let primary = tag.split('-').next().unwrap_or(tag).to_ascii_lowercase();
        match primary.as_str() {
            "zh" => Some(Language::Chinese),
            "en" => Some(Language::English),
            _ => None,
        }
    })
}

/// `q` 的值换成千分数；写错的整条不算。
fn quality_per_mille(value: &str) -> Option<u16> {
    let value: f32 = value.trim().parse().ok()?;
    if !(0.0..=1.0).contains(&value) {
        return None;
    }
    // 0 到 1000 之间，换成整数不会截断。
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some((value * 1_000.0).round() as u16)
}

/// 抓预览的爬虫多半不带 `Accept-Language`：房间名里有中文字就出中文页。
fn guess_language(name: &str) -> Language {
    let chinese = name.chars().any(|character| {
        matches!(character, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}')
    });
    if chinese {
        Language::Chinese
    } else {
        Language::English
    }
}

#[cfg(test)]
mod repository_tests;
#[cfg(test)]
mod tests;
