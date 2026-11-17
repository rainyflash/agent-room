//! 分享页的 HTML。房间名、简介、房主昵称都是用户写的，放进页面和 `og:` 标签之前一律经 [`escape`]。
//! 页面不知道看的人登没登录，按钮都去网页的 `/explore/<分享码>?join=1`，由网页接着登录、加入或申请。

use std::fmt::Write as _;

use agent_room_application::public_room_directory::PublicRoomDetailView;
use agent_room_domain::{room_listing::RoomJoinMode, time::UtcMillis};

/// 桌面应用去 GitHub 上最新的发行版下载，和网站一样。
pub(super) const LATEST_RELEASE: &str = "https://github.com/rainyflash/agent-room/releases/latest";
const DAY_MILLIS: i64 = 24 * 60 * 60 * 1_000;
/// `og:description` 最多取简介的前几个字。
const PREVIEW_CHARS: usize = 150;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Language {
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
    live: LiveWording,
    gone_title: &'static str,
    gone_text: &'static str,
    unavailable_title: &'static str,
    unavailable_text: &'static str,
    busy_title: &'static str,
    busy_text: &'static str,
}

/// 实况那一块的字。页面上的由 `share.js` 照着填，`{count}`、`{owner}` 由它换成数和名字。分享的一段对话
/// 那一页（`excerpt.rs`）在服务器上照同样的字写好每一句。
pub(super) struct LiveWording {
    heading: &'static str,
    read_only: &'static str,
    online: &'static str,
    online_at_least: &'static str,
    empty: &'static str,
    join: &'static str,
    pub(super) agent: &'static str,
    pub(super) network_agent: &'static str,
    pub(super) agent_of: &'static str,
    pub(super) withheld: &'static str,
    pub(super) file: &'static str,
    pub(super) truncated: &'static str,
    pub(super) edited: &'static str,
}

const CHINESE_LIVE: LiveWording = LiveWording {
    heading: "房间里正在说的话",
    read_only: "只读",
    online: "现在 {count} 个在线 · 只读",
    online_at_least: "现在至少 {count} 个在线 · 只读",
    empty: "开了实况以后还没人说话。",
    join: "加入房间就能说话。",
    agent: "Agent",
    network_agent: "网络 Agent",
    agent_of: "{owner} 的 Agent",
    withheld: "一条不公开的消息",
    file: "发了一个文件",
    truncated: "（太长了，只显示开头）",
    edited: "（改过）",
};

const ENGLISH_LIVE: LiveWording = LiveWording {
    heading: "What’s being said",
    read_only: "Read-only",
    online: "{count} online now · Read-only",
    online_at_least: "At least {count} online now · Read-only",
    empty: "No one has said anything since the live view was turned on.",
    join: "Join the room to talk.",
    agent: "Agent",
    network_agent: "Network agent",
    agent_of: "{owner}’s agent",
    withheld: "A message that isn’t shown publicly",
    file: "Shared a file",
    truncated: " (only the beginning is shown)",
    edited: " (edited)",
};

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
    live: CHINESE_LIVE,
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
    live: ENGLISH_LIVE,
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

/// 实况那一块的字，一段对话那一页也用。
pub(super) const fn live_wording(language: Language) -> &'static LiveWording {
    &text(language).live
}

/// “房间由用户创建，不审核内容”那一句。
pub(super) const fn disclaimer(language: Language) -> &'static str {
    text(language).disclaimer
}

/// 一间公开房间的分享页。`live` 时卡片下面放实况那一块；`image` 是这间房自己的预览图（3P-8a）。
pub(super) fn room_page(
    detail: &PublicRoomDetailView,
    live: bool,
    language: Language,
    origins: &Origins<'_>,
    now: UtcMillis,
    image: &str,
) -> String {
    let text = text(language);
    let room = &detail.room.room;
    let code = room.share_code.as_str();
    let title = format!("{} · Agent Room", room.name);
    let preview = preview(&room.description).unwrap_or_else(|| text.generic_description.to_owned());
    let mut body = String::new();
    body.push_str(&room_panel(detail, text, origins, now));
    if live {
        body.push_str(&live_panel(&text.live, code));
    }
    body.push_str(&agents_panel(text, origins, code));
    body.push_str(&steps_panel(text, origins));
    page(text, &title, &preview, Some(image), origins, &body)
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

/// 别的整页（仓库地址的“还没人认领”）也套这一层：同样的页头、`og:` 标签、样式和脚本。预览图是
/// 通用的那张。
pub(super) fn shell(
    language: Language,
    title: &str,
    description: &str,
    origins: &Origins<'_>,
    body: &str,
) -> String {
    page(text(language), title, description, None, origins, body)
}

/// 和 [`shell`] 一样，预览图换成 `image`（一段对话那一页自己的）。
pub(super) fn shell_with_image(
    language: Language,
    title: &str,
    description: &str,
    image: &str,
    origins: &Origins<'_>,
    body: &str,
) -> String {
    page(
        text(language),
        title,
        description,
        Some(image),
        origins,
        body,
    )
}

/// 网页上的通用预览图（`apps/web/public/room-share-preview-<语言>.png`）：没有自己的图的页面用它，
/// 自己的图画不了时也跳到它。
pub(super) fn generic_preview(app: &str, language: Language) -> String {
    format!("{app}/room-share-preview-{}.png", text(language).lang)
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
    page(text, title, text.generic_description, None, origins, &body)
}

fn page(
    text: &Wording,
    title: &str,
    description: &str,
    image: Option<&str>,
    origins: &Origins<'_>,
    body: &str,
) -> String {
    let title = escape(title);
    let description = escape(description);
    let app = escape(origins.app);
    let image = escape(&image.map_or_else(
        || generic_preview(origins.app, text.language),
        str::to_owned,
    ));
    format!(
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title}</title>\n<meta name=\"robots\" content=\"noindex\">\n\
         <meta name=\"description\" content=\"{description}\">\n\
         <meta property=\"og:type\" content=\"website\">\n\
         <meta property=\"og:site_name\" content=\"Agent Room\">\n\
         <meta property=\"og:title\" content=\"{title}\">\n\
         <meta property=\"og:description\" content=\"{description}\">\n\
         <meta property=\"og:image\" content=\"{image}\">\n\
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
         <div class=\"share-avatar\" data-tone=\"{tone}\" aria-hidden=\"true\">{initial}</div>\
         <div><h1>{name}</h1>\
         <p class=\"share-owner\">{owner_label} {owner}</p></div></div>",
        tone = room_tone(&room.catalog_id.to_string()),
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

/// 房间小图标的颜色按房间号挑，和网页的 `roomTone`（`apps/web/src/shared/ui/room-tone.ts`）是同一条
/// 算法：同一间房在分享页和应用里是同一种。改一边要改另一边，两边的测试写着同样的四个取值。
pub(super) fn room_tone(catalog_id: &str) -> &'static str {
    const TONES: [&str; 4] = ["teal", "coral", "sunny", "sky"];
    let hash = catalog_id.chars().fold(0_u32, |hash, character| {
        hash.wrapping_mul(31).wrapping_add(u32::from(character))
    });
    TONES[(hash % 4) as usize]
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

/// 实况那一块。先藏着，`share.js` 头一次问到 `/r/<码>/live` 才露出来，问到 404 再藏起来；没有脚本时
/// 页面上只有卡片。消息由脚本照着这里的字填，一律当纯文本放进去。
fn live_panel(text: &LiveWording, code: &str) -> String {
    format!(
        "<section class=\"share-panel share-live\" hidden data-live=\"/r/{code}/live\" \
         data-read-only=\"{read_only}\" data-online=\"{online}\" \
         data-online-at-least=\"{online_at_least}\" data-agent=\"{agent}\" \
         data-network-agent=\"{network_agent}\" data-agent-of=\"{agent_of}\" \
         data-withheld=\"{withheld}\" data-file=\"{file}\" data-truncated=\"{truncated}\" \
         data-edited=\"{edited}\"><div class=\"share-live-head\"><h2>{heading}</h2>\
         <p class=\"share-quiet\" data-live-status>{read_only}</p></div>\
         <ol class=\"share-live-messages\" data-live-messages></ol>\
         <p class=\"share-quiet\" data-live-empty hidden>{empty}</p>\
         <p class=\"share-live-join\">{join}</p></section>",
        code = escape(code),
        heading = escape(text.heading),
        read_only = escape(text.read_only),
        online = escape(text.online),
        online_at_least = escape(text.online_at_least),
        agent = escape(text.agent),
        network_agent = escape(text.network_agent),
        agent_of = escape(text.agent_of),
        withheld = escape(text.withheld),
        file = escape(text.file),
        truncated = escape(text.truncated),
        edited = escape(text.edited),
        empty = escape(text.empty),
        join = escape(text.join),
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
pub(crate) fn escape(text: &str) -> String {
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
