//! 分享页的 HTML。房间名、简介、房主昵称都是用户写的，放进页面和 `og:` 标签之前一律经 [`escape`]。
//! 页面不知道看的人登没登录，按钮都去网页的 `/explore/<分享码>?join=1`，由网页接着登录、加入或申请。

use std::fmt::Write as _;

use agent_room_application::public_room_directory::PublicRoomDetailView;
use agent_room_domain::{room_listing::RoomJoinMode, time::UtcMillis};

/// 桌面应用去 GitHub 上最新的发行版下载，和网站一样。
const LATEST_RELEASE: &str = "https://github.com/rainyflash/agent-room/releases/latest";
const DAY_MILLIS: i64 = 24 * 60 * 60 * 1_000;
/// `og:description` 最多取简介的前几个字。
const PREVIEW_CHARS: usize = 150;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Language {
    Chinese,
    English,
}

/// 页面用到的地址：分享链接所在的网站（裸域）和网页应用。
pub(super) struct Origins<'a> {
    pub(super) site: &'a str,
    pub(super) app: &'a str,
}

struct Wording {
    language: Language,
    lang: &'static str,
    generic_description: &'static str,
    explore: &'static str,
    owner: &'static str,
    public: &'static str,
    open: &'static str,
    request: &'static str,
    invite_only: &'static str,
    encrypted: &'static str,
    plain: &'static str,
    plain_note: &'static str,
    people: &'static str,
    agents: &'static str,
    online: &'static str,
    at_least: &'static str,
    age: &'static str,
    join_wide: &'static str,
    join_narrow: &'static str,
    ask: &'static str,
    no_account: &'static str,
    sign_up: &'static str,
    invite_only_text: &'static str,
    disclaimer: &'static str,
    agents_heading: &'static str,
    agents_text: &'static str,
    download_mac: &'static str,
    download_windows: &'static str,
    phone_text: &'static str,
    link_label: &'static str,
    copy: &'static str,
    copied: &'static str,
    selected: &'static str,
    steps_heading: &'static str,
    steps: [&'static str; 3],
    gone_title: &'static str,
    gone_text: &'static str,
    unavailable_title: &'static str,
    unavailable_text: &'static str,
    busy_title: &'static str,
    busy_text: &'static str,
}

const CHINESE: Wording = Wording {
    language: Language::Chinese,
    lang: "zh-CN",
    generic_description: "Agent Room 上的公开房间：人和 Agent 在这里一起聊。",
    explore: "看看别的公开房间",
    owner: "房主",
    public: "公开房间",
    open: "直接加入",
    request: "申请加入",
    invite_only: "只能邀请",
    encrypted: "端到端加密",
    plain: "不加密",
    plain_note: "这间房不加密：进来的人和服务器都能看到房间里的话。",
    people: "人",
    agents: "Agent",
    online: "现在在线",
    at_least: "至少",
    age: "开了",
    join_wide: "加入房间",
    join_narrow: "登录并加入",
    ask: "申请加入",
    no_account: "还没有账号？",
    sign_up: "用邮箱注册",
    invite_only_text: "这个房间只能由房主邀请。",
    disclaimer: "房间由用户创建，Agent Room 不审核内容。",
    agents_heading: "带上你的 Agent（可选）",
    agents_text: "加入只要网页，网页里就能聊。想把自己电脑上的 Agent 带进来，装上桌面应用，\
                  在“接入 Agent”里点一下。只在网页上用聊天助手的不用下载，登录后在“接入 Agent”\
                  里选“网络”。",
    download_mac: "下载 Mac 版",
    download_windows: "下载 Windows 版",
    phone_text: "在手机上？复制链接发到电脑上，再在电脑上打开。",
    link_label: "分享链接",
    copy: "复制链接",
    copied: "已复制",
    selected: "已经选中，自己复制一下",
    steps_heading: "怎么开始",
    steps: ["登录或注册", "加入房间", "带上 Agent（可选）"],
    gone_title: "这个房间现在不公开了",
    gone_text: "可能改回了私密、关了，或者分享链接换了。",
    unavailable_title: "暂时打不开",
    unavailable_text: "过一会儿再试一次。",
    busy_title: "这一会儿打开得太多了",
    busy_text: "等一分钟再打开。",
};

const ENGLISH: Wording = Wording {
    language: Language::English,
    lang: "en",
    generic_description: "A public room on Agent Room, where people and agents talk together.",
    explore: "Browse other public rooms",
    owner: "Hosted by",
    public: "Public room",
    open: "Anyone can join",
    request: "Ask to join",
    invite_only: "Invite only",
    encrypted: "End-to-end encrypted",
    plain: "Not encrypted",
    plain_note: "This room isn’t encrypted: everyone who joins, and the server, can read what’s \
                 said here.",
    people: "People",
    agents: "Agents",
    online: "Online now",
    at_least: "at least",
    age: "Open for",
    join_wide: "Join room",
    join_narrow: "Sign in and join",
    ask: "Ask to join",
    no_account: "No account yet?",
    sign_up: "Sign up with email",
    invite_only_text: "Only the host can invite people to this room.",
    disclaimer: "Rooms are created by users. Agent Room doesn’t review what’s in them.",
    agents_heading: "Bring your agent (optional)",
    agents_text: "A browser is all you need to join and chat. To bring an agent from your own \
                  computer, install the desktop app and use “Bring an agent”. If you use a chat \
                  assistant on the web, there’s nothing to download: after signing in, choose \
                  “Network” in “Bring an agent”.",
    download_mac: "Download for Mac",
    download_windows: "Download for Windows",
    phone_text: "On your phone? Copy the link and open it on your computer.",
    link_label: "Share link",
    copy: "Copy link",
    copied: "Copied",
    selected: "Selected — copy it from here",
    steps_heading: "How it works",
    steps: [
        "Sign in or sign up",
        "Join the room",
        "Bring your agent (optional)",
    ],
    gone_title: "This room isn’t public anymore",
    gone_text: "It may have been made private or closed, or its share link was changed.",
    unavailable_title: "Can’t open this right now",
    unavailable_text: "Please try again in a moment.",
    busy_title: "Too many requests",
    busy_text: "Please wait a minute and try again.",
};

const fn text(language: Language) -> &'static Wording {
    match language {
        Language::Chinese => &CHINESE,
        Language::English => &ENGLISH,
    }
}

/// 一间公开房间的分享页。
pub(super) fn room_page(
    detail: &PublicRoomDetailView,
    language: Language,
    origins: &Origins<'_>,
    now: UtcMillis,
) -> String {
    let text = text(language);
    let room = &detail.room.room;
    let code = room.share_code.as_str();
    let title = format!("{} · Agent Room", room.name);
    let preview = preview(&room.description).unwrap_or_else(|| text.generic_description.to_owned());
    let mut body = String::new();
    body.push_str(&room_panel(detail, text, origins, now));
    body.push_str(&agents_panel(text, origins, code));
    body.push_str(&steps_panel(text, origins));
    page(text, &title, &preview, origins, &body)
}

/// 不公开了、关了、码换了、没有这个码，都是这一页，不写房间名。
pub(super) fn gone_page(language: Language, origins: &Origins<'_>) -> String {
    let text = text(language);
    notice_page(text, origins, text.gone_title, text.gone_text)
}

/// 库或者 Matrix 暂时不通。
pub(super) fn unavailable_page(language: Language, origins: &Origins<'_>) -> String {
    let text = text(language);
    notice_page(text, origins, text.unavailable_title, text.unavailable_text)
}

/// 这个来源一分钟里打开得太多（`features/anonymous_limits.rs`）。
pub(super) fn busy_page(language: Language, origins: &Origins<'_>) -> String {
    let text = text(language);
    notice_page(text, origins, text.busy_title, text.busy_text)
}

fn notice_page(text: &Wording, origins: &Origins<'_>, title: &str, message: &str) -> String {
    let body = format!(
        "<section class=\"share-panel\"><h1>{title}</h1><p class=\"share-quiet\">{message}</p>\
         <div class=\"share-actions\"><a class=\"share-button share-button--primary\" \
         href=\"{app}/explore\">{explore}</a></div></section>",
        title = escape(title),
        message = escape(message),
        app = escape(origins.app),
        explore = escape(text.explore),
    );
    page(text, title, text.generic_description, origins, &body)
}

fn page(
    text: &Wording,
    title: &str,
    description: &str,
    origins: &Origins<'_>,
    body: &str,
) -> String {
    let title = escape(title);
    let description = escape(description);
    let app = escape(origins.app);
    format!(
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title}</title>\n<meta name=\"robots\" content=\"noindex\">\n\
         <meta name=\"description\" content=\"{description}\">\n\
         <meta property=\"og:type\" content=\"website\">\n\
         <meta property=\"og:site_name\" content=\"Agent Room\">\n\
         <meta property=\"og:title\" content=\"{title}\">\n\
         <meta property=\"og:description\" content=\"{description}\">\n\
         <meta property=\"og:image\" content=\"{app}/room-share-preview-{lang}.png\">\n\
         <meta property=\"og:image:width\" content=\"1280\">\n\
         <meta property=\"og:image:height\" content=\"640\">\n\
         <meta name=\"twitter:card\" content=\"summary_large_image\">\n\
         <link rel=\"stylesheet\" href=\"/r/_/share.css\">\n\
         <script src=\"/r/_/share.js\" defer></script>\n</head>\n<body>\n\
         <header class=\"share-header\"><a class=\"share-brand\" href=\"/\">Agent Room</a>\
         <a href=\"{app}/explore\">{explore}</a></header>\n\
         <main class=\"share-main\">{body}</main>\n</body>\n</html>\n",
        lang = text.lang,
        explore = escape(text.explore),
    )
}

fn room_panel(
    detail: &PublicRoomDetailView,
    text: &Wording,
    origins: &Origins<'_>,
    now: UtcMillis,
) -> String {
    let room = &detail.room.room;
    let initial = room
        .name
        .chars()
        .next()
        .map(String::from)
        .unwrap_or_default();
    let mut panel = format!(
        "<section class=\"share-panel\"><div class=\"share-room-head\">\
         <div class=\"share-avatar\" aria-hidden=\"true\">{initial}</div><div><h1>{name}</h1>\
         <p class=\"share-owner\">{owner_label} {owner}</p></div></div>",
        initial = escape(&initial),
        name = escape(&room.name),
        owner_label = escape(text.owner),
        owner = escape(&room.owner_display_name),
    );
    let (join_label, join_class) = match room.join_mode {
        RoomJoinMode::Open => (text.open, " class=\"share-tag--open\""),
        RoomJoinMode::Request => (text.request, ""),
        RoomJoinMode::InviteOnly => (text.invite_only, ""),
    };
    let encryption = if room.end_to_end_encrypted {
        format!("<li>{}</li>", escape(text.encrypted))
    } else {
        format!("<li class=\"share-tag--plain\">{}</li>", escape(text.plain))
    };
    let _ = write!(
        panel,
        "<ul class=\"share-tags\"><li>{public}</li><li{join_class}>{join}</li>{encryption}</ul>",
        public = escape(text.public),
        join = escape(join_label),
    );
    if !room.description.trim().is_empty() {
        let _ = write!(
            panel,
            "<p class=\"share-description\">{}</p>",
            escape(room.description.trim())
        );
    }
    panel.push_str(&stats(detail, text, now));
    if !room.end_to_end_encrypted {
        let _ = write!(
            panel,
            "<p class=\"share-note\">{}</p>",
            escape(text.plain_note)
        );
    }
    panel.push_str(&actions(
        room.join_mode,
        text,
        origins,
        room.share_code.as_str(),
    ));
    let _ = write!(
        panel,
        "<p class=\"share-disclaimer\">{}</p></section>",
        escape(text.disclaimer)
    );
    panel
}

fn stats(detail: &PublicRoomDetailView, text: &Wording, now: UtcMillis) -> String {
    let room = &detail.room.room;
    let mut stats = String::from("<dl class=\"share-stats\">");
    let mut item = |label: &str, value: &str| {
        let _ = write!(
            stats,
            "<div><dt>{}</dt><dd>{}</dd></div>",
            escape(label),
            escape(value)
        );
    };
    item(text.people, &room.member_count.to_string());
    item(text.agents, &room.agent_count.to_string());
    if let Some(online) = detail.online {
        let count = if online.at_least {
            format!("{} {}", text.at_least, online.count)
        } else {
            online.count.to_string()
        };
        item(text.online, &count);
    }
    item(text.age, &age(room.created_at, now, text));
    stats.push_str("</dl>");
    stats
}

fn actions(join_mode: RoomJoinMode, text: &Wording, origins: &Origins<'_>, code: &str) -> String {
    let join = format!("{}/explore/{code}?join=1", origins.app);
    let sign_up = format!("{join}&signup=1");
    let button = match join_mode {
        RoomJoinMode::InviteOnly => {
            return format!(
                "<p class=\"share-quiet\">{}</p>",
                escape(text.invite_only_text)
            );
        }
        RoomJoinMode::Open => format!(
            "<span class=\"share-wide\">{}</span><span class=\"share-narrow\">{}</span>",
            escape(text.join_wide),
            escape(text.join_narrow)
        ),
        RoomJoinMode::Request => escape(text.ask),
    };
    format!(
        "<div class=\"share-actions\"><a class=\"share-button share-button--primary\" \
         href=\"{join}\">{button}</a><p class=\"share-quiet\">{no_account} \
         <a href=\"{sign_up}\">{sign_up_label}</a></p></div>",
        join = escape(&join),
        sign_up = escape(&sign_up),
        no_account = escape(text.no_account),
        sign_up_label = escape(text.sign_up),
    )
}

fn agents_panel(text: &Wording, origins: &Origins<'_>, code: &str) -> String {
    let link = super::share_url(origins.site, code);
    format!(
        "<section class=\"share-panel\"><h2>{heading}</h2><p>{body}</p>\
         <div class=\"share-row share-wide\">\
         <a class=\"share-button\" href=\"{release}\">{mac}</a>\
         <a class=\"share-button\" href=\"{release}\">{windows}</a></div>\
         <p class=\"share-narrow share-quiet\">{phone}</p>\
         <div class=\"share-row\"><input class=\"share-link\" id=\"share-link\" type=\"text\" \
         readonly value=\"{link}\" aria-label=\"{link_label}\">\
         <button class=\"share-button\" type=\"button\" data-copy-link=\"share-link\" \
         data-copied=\"{copied}\" data-select=\"{selected}\">{copy}</button></div>\
         <p class=\"share-quiet\" data-copy-status role=\"status\"></p></section>",
        heading = escape(text.agents_heading),
        body = escape(text.agents_text),
        release = LATEST_RELEASE,
        mac = escape(text.download_mac),
        windows = escape(text.download_windows),
        phone = escape(text.phone_text),
        link = escape(&link),
        link_label = escape(text.link_label),
        copied = escape(text.copied),
        selected = escape(text.selected),
        copy = escape(text.copy),
    )
}

fn steps_panel(text: &Wording, origins: &Origins<'_>) -> String {
    let mut steps = String::new();
    for step in text.steps {
        let _ = write!(steps, "<li>{}</li>", escape(step));
    }
    format!(
        "<section class=\"share-panel\"><h2>{heading}</h2><ol class=\"share-steps\">{steps}</ol>\
         <p><a href=\"{app}/explore\">{explore}</a></p></section>",
        heading = escape(text.steps_heading),
        app = escape(origins.app),
        explore = escape(text.explore),
    )
}

/// 简介的开头压成一行，给 `og:description`。没写简介是 `None`。
fn preview(description: &str) -> Option<String> {
    let line = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= PREVIEW_CHARS {
        return Some(line);
    }
    let mut short: String = line.chars().take(PREVIEW_CHARS).collect();
    short.push('…');
    Some(short)
}

/// 房间开了多久：不到一天、几天、几个月、几年。
fn age(created_at: UtcMillis, now: UtcMillis, text: &Wording) -> String {
    let days = (now.value() - created_at.value()).max(0) / DAY_MILLIS;
    let chinese = text.language == Language::Chinese;
    let (count, unit_zh, unit_en) = match days {
        0 => {
            return if chinese {
                "不到一天".to_owned()
            } else {
                "Less than a day".to_owned()
            };
        }
        1..=59 => (days, "天", "day"),
        60..=729 => (days / 30, "个月", "month"),
        _ => (days / 365, "年", "year"),
    };
    if chinese {
        format!("{count} {unit_zh}")
    } else if count == 1 {
        format!("1 {unit_en}")
    } else {
        format!("{count} {unit_en}s")
    }
}

/// HTML 正文和属性里都能用：五个特殊字符都换掉。
pub(super) fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            character => escaped.push(character),
        }
    }
    escaped
}
