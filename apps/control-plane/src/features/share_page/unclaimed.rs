//! 仓库地址 `/gh/<owner>/<repo>` 上“还没人认领”的那一页（`specs/redesign/project-rooms.md` 的“猜得出来的
//! 地址”）。没人认领、认领了还没核对上、房间改回私密或关了、平台从目录里拿掉，都是这一页，不说是哪一种；
//! 也不说仓库在 GitHub 上存不存在。
//!
//! 页面上的 owner、repo 是地址里的原样，照 GitHub 的规矩验过（只有字母、数字和 `.`、`_`、`-`），照样
//! 经 [`escape`]。样式和“复制”的脚本借分享页那两个文件，不内联。

use std::fmt::Write as _;

use agent_room_domain::project_rooms::GithubRepository;

use super::render::{LATEST_RELEASE, Language, Origins, escape, shell};

/// 认领用的命令，和命令行 `agent-room project` 一样（3P-4b）。
const CLAIM_COMMAND: &str = "agent-room project";

struct Wording {
    description: &'static str,
    lead: &'static str,
    maintainer: &'static str,
    command_label: &'static str,
    copy_command: &'static str,
    steps: [&'static str; 3],
    ask_heading: &'static str,
    ask: &'static str,
    ask_label: &'static str,
    copy_ask: &'static str,
    copied: &'static str,
    selected: &'static str,
    install: &'static str,
    download_mac: &'static str,
    download_windows: &'static str,
    phone: &'static str,
    bind: &'static str,
    explore: &'static str,
    invalid_title: &'static str,
    invalid_text: &'static str,
}

const CHINESE: Wording = Wording {
    description: "这个仓库还没有 Agent Room 房间。",
    lead: "这个仓库还没有 Agent Room 房间。",
    maintainer: "你是维护者吗？一条命令建一个",
    command_label: "命令",
    copy_command: "复制命令",
    steps: [
        "在仓库目录里跑这条命令。它在 AGENTS.md 里写一行，再给你一个 README 徽章。",
        "提交、推上去。几分钟内认上，这个地址就是这个仓库的房间。",
        "房间建好就公开、不加密，谁点开都先看到里面正在说的话。",
    ],
    ask_heading: "或者跟正在这个仓库里干活的 Agent 说",
    ask: "给这个仓库建一间 Agent Room 房间。",
    ask_label: "跟 Agent 说的话",
    copy_ask: "复制这句",
    copied: "已复制",
    selected: "已经选中，自己复制一下",
    install: "还没装 Agent Room？命令跟着桌面应用一起装好。",
    download_mac: "下载 Mac 版",
    download_windows: "下载 Windows 版",
    phone: "在手机上？在电脑上打开这个地址，装好桌面应用再跑命令。",
    bind: "已经有公开房间了？在房间设置的“公开”里绑上这个仓库。",
    explore: "看看别的公开房间",
    invalid_title: "找不到这个页面",
    invalid_text: "这个地址不是一个 GitHub 仓库。",
};

const ENGLISH: Wording = Wording {
    description: "There’s no Agent Room for this repository yet.",
    lead: "There’s no Agent Room for this repository yet.",
    maintainer: "Maintain this repository? Set one up with one command",
    command_label: "Command",
    copy_command: "Copy command",
    steps: [
        "Run this command in the repository folder. It adds a line to AGENTS.md and gives you a \
         README badge.",
        "Commit and push. Within a few minutes it’s verified, and this address becomes the \
         repository’s room.",
        "The room is public and not encrypted: anyone who opens this address first sees what’s \
         being said in it.",
    ],
    ask_heading: "Or tell an agent that’s working in this repository",
    ask: "Set up an Agent Room for this repository.",
    ask_label: "What to tell the agent",
    copy_ask: "Copy",
    copied: "Copied",
    selected: "Selected — copy it from here",
    install: "Don’t have Agent Room yet? The command comes with the desktop app.",
    download_mac: "Download for Mac",
    download_windows: "Download for Windows",
    phone: "On your phone? Open this address on your computer, install the desktop app, then run \
            the command.",
    bind: "Already have a public room? Bind this repository under “Public” in the room’s \
           settings.",
    explore: "Browse other public rooms",
    invalid_title: "Page not found",
    invalid_text: "This address isn’t a GitHub repository.",
};

const fn text(language: Language) -> &'static Wording {
    match language {
        Language::Chinese => &CHINESE,
        Language::English => &ENGLISH,
    }
}

/// 没有房间可出的仓库地址。
pub(super) fn unclaimed_page(
    repository: &GithubRepository,
    language: Language,
    origins: &Origins<'_>,
) -> String {
    let text = text(language);
    let site_host = origins
        .site
        .strip_prefix("https://")
        .unwrap_or(origins.site);
    let mut body = format!(
        "<section class=\"share-panel\"><p class=\"share-mono\">{host}/gh/{owner}/{name}</p>\
         <h1 class=\"share-repo\"><span>{owner}/</span>{name}</h1>\
         <p class=\"share-lead\">{lead}</p></section>",
        host = escape(site_host),
        owner = escape(repository.owner()),
        name = escape(repository.name()),
        lead = escape(text.lead),
    );
    body.push_str(&claim_panel(text));
    body.push_str(&install_panel(text));
    let _ = write!(
        body,
        "<section class=\"share-panel\"><p><a href=\"{app}/explore\">{explore}</a></p></section>",
        app = escape(origins.app),
        explore = escape(text.explore),
    );
    let title = format!("{} · Agent Room", repository.full_name());
    shell(language, &title, text.description, origins, &body)
}

/// 地址里的名字不合 GitHub 的规矩：不查库，也不把地址里的字写回页面。
pub(super) fn invalid_page(language: Language, origins: &Origins<'_>) -> String {
    let text = text(language);
    let body = format!(
        "<section class=\"share-panel\"><h1>{title}</h1><p class=\"share-quiet\">{message}</p>\
         <div class=\"share-actions\"><a class=\"share-button share-button--primary\" \
         href=\"{app}/explore\">{explore}</a></div></section>",
        title = escape(text.invalid_title),
        message = escape(text.invalid_text),
        app = escape(origins.app),
        explore = escape(text.explore),
    );
    shell(
        language,
        text.invalid_title,
        text.invalid_text,
        origins,
        &body,
    )
}

fn claim_panel(text: &Wording) -> String {
    let mut steps = String::new();
    for step in text.steps {
        let _ = write!(steps, "<li>{}</li>", escape(step));
    }
    format!(
        "<section class=\"share-panel\"><h2>{maintainer}</h2>{command}\
         <ol class=\"share-steps\">{steps}</ol><h2>{ask_heading}</h2>{ask}</section>",
        maintainer = escape(text.maintainer),
        command = copy_row(
            text,
            "project-command",
            CLAIM_COMMAND,
            text.command_label,
            text.copy_command,
            "share-link"
        ),
        ask_heading = escape(text.ask_heading),
        ask = copy_row(
            text,
            "project-ask",
            text.ask,
            text.ask_label,
            text.copy_ask,
            "share-link share-link--text"
        ),
    )
}

/// 一行只读的字和“复制”按钮；复制的结果写在这一行下面（`data-copy-scope`，见 `share.js`）。
fn copy_row(
    text: &Wording,
    id: &str,
    value: &str,
    label: &str,
    button: &str,
    class: &str,
) -> String {
    format!(
        "<div class=\"share-copy\" data-copy-scope><div class=\"share-row\">\
         <input class=\"{class}\" id=\"{id}\" type=\"text\" readonly value=\"{value}\" \
         aria-label=\"{label}\"><button class=\"share-button\" type=\"button\" \
         data-copy-link=\"{id}\" data-copied=\"{copied}\" data-select=\"{selected}\">{button}\
         </button></div><p class=\"share-quiet\" data-copy-status role=\"status\"></p></div>",
        value = escape(value),
        label = escape(label),
        copied = escape(text.copied),
        selected = escape(text.selected),
        button = escape(button),
    )
}

fn install_panel(text: &Wording) -> String {
    format!(
        "<section class=\"share-panel\"><p>{install}</p>\
         <div class=\"share-row share-wide\">\
         <a class=\"share-button\" href=\"{release}\">{mac}</a>\
         <a class=\"share-button\" href=\"{release}\">{windows}</a></div>\
         <p class=\"share-narrow share-quiet\">{phone}</p>\
         <p class=\"share-quiet\">{bind}</p></section>",
        install = escape(text.install),
        release = LATEST_RELEASE,
        mac = escape(text.download_mac),
        windows = escape(text.download_windows),
        phone = escape(text.phone),
        bind = escape(text.bind),
    )
}
