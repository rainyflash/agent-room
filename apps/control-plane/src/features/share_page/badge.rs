//! README 徽章 `/gh/<owner>/<repo>/badge.svg`（`specs/redesign/project-rooms.md` 的“README 徽章”）：左边黑底
//! “Agent Room”带一个小机器人，右边白底写现在几个在线（`?lang=zh-CN` 写“N 在线”）。没绑房间、房间不公开
//! 了都是“no room”（“没有房间”），同一张，不说为什么。
//!
//! README 里的图都经 GitHub 的图片代理来取，来取的就那几个地址，所以不按来源限流，靠缓存扛：
//! - 每个仓库算出来的数在内存里留 60 秒，“没有房间”留 5 分钟（防随便编仓库名打库）；回答带
//!   `Cache-Control: public, max-age=300` 和 `ETag`，认 `If-None-Match`。
//! - 内存里没有时，同一个仓库同时只算一次，别的请求等它。
//! - 全站每秒最多算 50 个新的，超了直接给不带数的徽章，不碰库和聊天服务器。
//!
//! 图里除了算出来的数，没有一个字是从请求或库里来的：仓库名、房间名都不进图。

use std::{
    collections::HashMap,
    fmt::Write as _,
    hash::{DefaultHasher, Hash as _, Hasher as _},
    sync::{Arc, Mutex, PoisonError},
};

use agent_room_application::{
    ports::Clock, project_rooms::RepositoryRoomPages,
    public_room_directory::PublicDirectoryFailureKind,
};
use agent_room_domain::project_rooms::GithubRepository;
use axum::{
    extract::{Path, RawQuery, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures_util::future::{BoxFuture, FutureExt as _, Shared};

use super::SharePageHttpState;

/// 算出来在线数的留多久。
pub(super) const COUNT_TTL_MILLIS: i64 = 60_000;
/// “没有房间”留多久。
pub(super) const NO_ROOM_TTL_MILLIS: i64 = 5 * 60_000;
/// 全站每秒最多算几个新的。
pub(super) const NEW_PER_SECOND: u32 = 50;
/// 最多记着几个仓库。满了先扔掉过期的；还满就只留正在算的。每秒最多 50 个新的，五分钟也才一万五。
const MAX_SLOTS: usize = 20_000;
const BADGE_CACHE: &str = "public, max-age=300";
/// 不带数的那张（太忙、库读不了）：别让图片代理记住它。
const FALLBACK_CACHE: &str = "no-store";

/// 一个仓库查出来的样子。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lookup {
    Online {
        count: u32,
        at_least: bool,
    },
    /// 绑着、公开着，在线数问不到。
    Unknown,
    NoRoom,
    /// 库读不了：不记住，下一个请求再算。
    Failed,
    /// 这一秒算新的额度用完了。
    Busy,
}

impl Lookup {
    const fn ttl_millis(self) -> i64 {
        match self {
            Self::Online { .. } | Self::Unknown => COUNT_TTL_MILLIS,
            Self::NoRoom => NO_ROOM_TTL_MILLIS,
            Self::Failed | Self::Busy => 0,
        }
    }

    /// 能不能让图片代理缓存、带 `ETag`。
    const fn cacheable(self) -> bool {
        !matches!(self, Self::Failed | Self::Busy)
    }
}

type Flight = Shared<BoxFuture<'static, Lookup>>;

enum Slot {
    Ready { at: i64, lookup: Lookup },
    Pending(Flight),
}

#[derive(Default)]
struct Ledger {
    slots: HashMap<String, Slot>,
    /// 这一秒（`second`）已经开始算了几个新的。
    second: i64,
    started: u32,
}

/// 徽章上的数：每个仓库的缓存、同一个仓库只算一次、全站每秒的额度。只在这个进程的内存里。
pub(crate) struct BadgeCounts {
    pages: Arc<dyn RepositoryRoomPages>,
    clock: Arc<dyn Clock>,
    ledger: Mutex<Ledger>,
}

impl BadgeCounts {
    pub(crate) fn new(pages: Arc<dyn RepositoryRoomPages>, clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self {
            pages,
            clock,
            ledger: Mutex::new(Ledger::default()),
        })
    }

    pub(super) async fn lookup(self: &Arc<Self>, repository: GithubRepository) -> Lookup {
        let key = repository.full_name().to_ascii_lowercase();
        let flight = {
            let now = self.clock.now().value();
            let mut ledger = self.ledger.lock().unwrap_or_else(PoisonError::into_inner);
            match ledger.slots.get(&key) {
                Some(Slot::Ready { at, lookup }) if fresh(*at, *lookup, now) => return *lookup,
                Some(Slot::Pending(flight)) => flight.clone(),
                _ => {
                    if !ledger.spend(now) {
                        return Lookup::Busy;
                    }
                    if !ledger.slots.contains_key(&key) && ledger.slots.len() >= MAX_SLOTS {
                        ledger.prune(now);
                    }
                    let flight = self.start(key.clone(), repository);
                    ledger.slots.insert(key, Slot::Pending(flight.clone()));
                    flight
                }
            }
        };
        flight.await
    }

    /// 另起一个任务去算、记下结果：等着的请求都走了也照样算完，不会留下一个一直在等的仓库。调用的时候
    /// 拿着 `ledger` 的锁，任务记结果要等这把锁，所以总在“正在算”记上以后才记。
    fn start(self: &Arc<Self>, key: String, repository: GithubRepository) -> Flight {
        let counts = Arc::clone(self);
        let settled_key = key.clone();
        let task = tokio::spawn(async move {
            let lookup = counts.compute(repository).await;
            counts.settle(&settled_key, lookup);
            lookup
        });
        let counts = Arc::clone(self);
        async move {
            task.await.unwrap_or_else(|_| {
                // 任务没算完就没了：别让这个仓库一直“正在算”。
                counts.settle(&key, Lookup::Failed);
                Lookup::Failed
            })
        }
        .boxed()
        .shared()
    }

    async fn compute(&self, repository: GithubRepository) -> Lookup {
        match self.pages.public_room(repository).await {
            Ok(detail) => detail
                .online
                .map_or(Lookup::Unknown, |online| Lookup::Online {
                    count: online.count,
                    at_least: online.at_least,
                }),
            Err(failure) => match failure.kind() {
                PublicDirectoryFailureKind::NotFound
                | PublicDirectoryFailureKind::InvalidRequest => Lookup::NoRoom,
                PublicDirectoryFailureKind::DependencyUnavailable
                | PublicDirectoryFailureKind::Internal => {
                    tracing::info!(
                        operation = failure.operation(),
                        failure = ?failure.kind(),
                        "徽章没读到房间"
                    );
                    Lookup::Failed
                }
            },
        }
    }

    fn settle(&self, key: &str, lookup: Lookup) {
        let now = self.clock.now().value();
        let mut ledger = self.ledger.lock().unwrap_or_else(PoisonError::into_inner);
        if lookup.ttl_millis() > 0 {
            ledger
                .slots
                .insert(key.to_owned(), Slot::Ready { at: now, lookup });
        } else {
            ledger.slots.remove(key);
        }
    }
}

impl Ledger {
    /// 这一秒还能不能再算一个新的；能就记上。
    fn spend(&mut self, now: i64) -> bool {
        let second = now.div_euclid(1_000);
        if second != self.second {
            self.second = second;
            self.started = 0;
        }
        if self.started >= NEW_PER_SECOND {
            return false;
        }
        self.started += 1;
        true
    }

    fn prune(&mut self, now: i64) {
        self.slots.retain(|_, slot| match slot {
            Slot::Ready { at, lookup } => fresh(*at, *lookup, now),
            Slot::Pending(_) => true,
        });
        if self.slots.len() >= MAX_SLOTS {
            self.slots
                .retain(|_, slot| matches!(slot, Slot::Pending(_)));
        }
    }
}

fn fresh(at: i64, lookup: Lookup, now: i64) -> bool {
    now.saturating_sub(at) < lookup.ttl_millis()
}

pub(super) async fn badge(
    State(state): State<SharePageHttpState>,
    Path((owner, name)): Path<(String, String)>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    // 名字不合 GitHub 的规矩：不查库。
    let Ok(repository) = GithubRepository::new(&owner, &name) else {
        return (
            StatusCode::NOT_FOUND,
            [(header::CACHE_CONTROL, "no-store")],
            "",
        )
            .into_response();
    };
    let language = BadgeLanguage::from_query(query.as_deref());
    let lookup = state.badges.lookup(repository).await;
    respond(&headers, lookup, &svg(lookup, language))
}

fn respond(request: &HeaderMap, lookup: Lookup, svg: &str) -> Response {
    let cacheable = lookup.cacheable();
    let etag = cacheable.then(|| etag_of(svg));
    let not_modified = etag
        .as_deref()
        .is_some_and(|etag| matches_if_none_match(request, etag));
    let mut response = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (StatusCode::OK, svg.to_owned()).into_response()
    };
    let headers = response.headers_mut();
    if !not_modified {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("image/svg+xml; charset=utf-8"),
        );
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if cacheable {
            BADGE_CACHE
        } else {
            FALLBACK_CACHE
        }),
    );
    if let Some(value) = etag.and_then(|etag| HeaderValue::from_str(&etag).ok()) {
        headers.insert(header::ETAG, value);
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

/// 图的内容一变 `ETag` 就变：样子改了不用记着换。
fn etag_of(svg: &str) -> String {
    let mut hasher = DefaultHasher::new();
    svg.hash(&mut hasher);
    format!("\"{:016x}\"", hasher.finish())
}

/// `If-None-Match` 里有这个 `ETag`（弱比较，`W/` 不论）或者 `*`。
pub(super) fn matches_if_none_match(request: &HeaderMap, etag: &str) -> bool {
    request
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .any(|candidate| candidate == "*" || candidate.trim_start_matches("W/") == etag)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BadgeLanguage {
    English,
    Chinese,
}

impl BadgeLanguage {
    /// 默认英文；`?lang=zh-CN`（`zh` 开头的都算）是中文。
    fn from_query(query: Option<&str>) -> Self {
        let chinese = query.is_some_and(|query| {
            url::form_urlencoded::parse(query.as_bytes()).any(|(key, value)| {
                key == "lang"
                    && value
                        .split(['-', '_'])
                        .next()
                        .is_some_and(|primary| primary.eq_ignore_ascii_case("zh"))
            })
        });
        if chinese {
            Self::Chinese
        } else {
            Self::English
        }
    }
}

const LABEL: &str = "Agent Room";
const HEIGHT: u32 = 20;
const PADDING: u32 = 6;
/// 机器人占的宽度，加上它和字之间的空。
const ROBOT: u32 = 18;
const INK: &str = "#18181b";
const PAPER: &str = "#ffffff";
/// 机器人的颜色：颜色只给机器人。
const ROBOT_COLOR: &str = "#8fe3c8";

/// 右边那段写什么；不带数的那张没有右边。
fn value_text(lookup: Lookup, language: BadgeLanguage) -> Option<String> {
    let (online, no_room) = match language {
        BadgeLanguage::English => (" online", "no room"),
        BadgeLanguage::Chinese => (" 在线", "没有房间"),
    };
    match lookup {
        Lookup::Online { count, at_least } => {
            let plus = if at_least { "+" } else { "" };
            Some(format!("{count}{plus}{online}"))
        }
        Lookup::NoRoom => Some(no_room.to_owned()),
        Lookup::Unknown | Lookup::Failed | Lookup::Busy => None,
    }
}

/// 一张徽章：20 像素高，两段。
fn svg(lookup: Lookup, language: BadgeLanguage) -> String {
    let value = value_text(lookup, language);
    let label_width = text_width(LABEL);
    let left = PADDING + ROBOT + label_width + PADDING;
    let right = value
        .as_deref()
        .map_or(0, |value| PADDING + text_width(value) + PADDING);
    let width = left + right;
    let aria = value
        .as_deref()
        .map_or_else(|| LABEL.to_owned(), |value| format!("{LABEL}: {value}"));
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{HEIGHT}\" \
         viewBox=\"0 0 {width} {HEIGHT}\" role=\"img\" aria-label=\"{aria}\">\
         <title>{aria}</title>\
         <clipPath id=\"badge\"><rect width=\"{width}\" height=\"{HEIGHT}\" rx=\"3\"/></clipPath>\
         <g clip-path=\"url(#badge)\"><rect width=\"{left}\" height=\"{HEIGHT}\" fill=\"{INK}\"/>"
    );
    if right > 0 {
        let _ = write!(
            svg,
            "<rect x=\"{left}\" width=\"{right}\" height=\"{HEIGHT}\" fill=\"{PAPER}\"/></g>\
             <rect x=\"0.5\" y=\"0.5\" width=\"{inner}\" height=\"19\" rx=\"2.5\" fill=\"none\" \
             stroke=\"{INK}\"/>",
            inner = width - 1,
        );
    } else {
        svg.push_str("</g>");
    }
    svg.push_str(&robot());
    let _ = write!(
        svg,
        "<g font-family=\"Verdana,Geneva,DejaVu Sans,PingFang SC,Microsoft YaHei,sans-serif\" \
         font-size=\"11\"><text x=\"{x}\" y=\"14\" fill=\"{PAPER}\" textLength=\"{label_width}\" \
         lengthAdjust=\"spacingAndGlyphs\">{LABEL}</text>",
        x = PADDING + ROBOT,
    );
    if let Some(value) = value {
        let _ = write!(
            svg,
            "<text x=\"{x}\" y=\"14\" fill=\"{INK}\" textLength=\"{length}\" \
             lengthAdjust=\"spacingAndGlyphs\">{value}</text>",
            x = left + PADDING,
            length = text_width(&value),
            value = super::escape(&value),
        );
    }
    svg.push_str("</g></svg>");
    svg
}

/// 14 像素见方的小机器人：天线、圆角的头、两只耳朵、两只眼睛。
fn robot() -> String {
    format!(
        "<g fill=\"{ROBOT_COLOR}\"><circle cx=\"13\" cy=\"3.4\" r=\"1.3\"/>\
         <rect x=\"12.4\" y=\"3.4\" width=\"1.2\" height=\"2.6\"/>\
         <rect x=\"7\" y=\"5.6\" width=\"12\" height=\"9.4\" rx=\"3\"/>\
         <rect x=\"5.4\" y=\"8.6\" width=\"1.6\" height=\"3.2\" rx=\"0.8\"/>\
         <rect x=\"19\" y=\"8.6\" width=\"1.6\" height=\"3.2\" rx=\"0.8\"/></g>\
         <g fill=\"{INK}\"><circle cx=\"10.6\" cy=\"10.2\" r=\"1.4\"/>\
         <circle cx=\"15.4\" cy=\"10.2\" r=\"1.4\"/></g>"
    )
}

/// 估一段字在 11 像素的 Verdana 下有多宽（取整到像素）。图里用 `textLength` 把字撑到这个宽度，估得
/// 不准也不会出框。
fn text_width(text: &str) -> u32 {
    let tenths: u32 = text
        .chars()
        .map(|character| match character {
            ' ' => 39,
            'i' | 'l' | 'I' | '.' | ',' | ':' | ';' | '\'' | '|' | '!' => 33,
            'f' | 'j' | 'r' | 't' => 45,
            'm' | 'w' | 'M' | 'W' => 105,
            '+' => 92,
            '0'..='9' => 70,
            'A'..='Z' => 75,
            character if character.is_ascii() => 67,
            _ => 110,
        })
        .sum();
    tenths.div_ceil(10)
}
