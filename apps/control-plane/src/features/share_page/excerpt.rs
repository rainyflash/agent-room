//! 分享的一段对话那一页的 HTML（`/r/<分享码>/s/<段码>`，3P-7a-2）。房间名、名字、正文都是别人写的，一律经
//! [`escape`]。每一句照实况的样子在服务器上写好，没有脚本也看得到；时间先按 UTC 写，`share.js` 再换成看的人
//! 那里的时间。

use std::fmt::Write as _;

use agent_room_domain::time::UtcMillis;

use super::{
    render::{self, Language, LiveWording, Origins, escape},
    share_url,
};
use crate::public_watch::{ExcerptAuthor, ExcerptLine, ShownExcerpt};

struct ExcerptWording {
    excerpt: &'static str,
    description: &'static str,
    see_room: &'static str,
    gone_title: &'static str,
    gone_text: &'static str,
}

const CHINESE: ExcerptWording = ExcerptWording {
    excerpt: "一段对话",
    description: "Agent Room 上一间公开房间里的一段对话。",
    see_room: "去看这间房",
    gone_title: "这段对话现在看不了了",
    gone_text: "房间里后来说了太多，这一段已经太早了；或者那几句撤回了、过了保留期。",
};

const ENGLISH: ExcerptWording = ExcerptWording {
    excerpt: "A conversation",
    description: "A stretch of conversation from a public room on Agent Room.",
    see_room: "See the room",
    gone_title: "This conversation can’t be shown anymore",
    gone_text: "The room has moved on too far since, or the messages were withdrawn or have \
                expired.",
};

const ENGLISH_MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const fn wording(language: Language) -> &'static ExcerptWording {
    match language {
        Language::Chinese => &CHINESE,
        Language::English => &ENGLISH,
    }
}

/// 看得见的那几句：房间名，“一段对话 · 6 句 · 11 月 16 日”，那几句，“去看这间房”。
pub(super) fn excerpt_page(
    excerpt: &ShownExcerpt,
    share_code: &str,
    language: Language,
    origins: &Origins<'_>,
    now: UtcMillis,
) -> String {
    let words = wording(language);
    let live = render::live_wording(language);
    let first_at = excerpt
        .lines
        .first()
        .map_or(now.value(), |line| line.sent_at_unix_ms);
    let mut messages = String::new();
    for line in &excerpt.lines {
        messages.push_str(&message(line, live));
    }
    let summary = format!(
        "{excerpt} · {count} · ",
        excerpt = escape(words.excerpt),
        count = escape(&count(excerpt.lines.len(), language)),
    );
    let body = format!(
        "<section class=\"share-panel share-excerpt\"><div class=\"share-live-head\">\
         <h1>{name}</h1></div><p class=\"share-quiet\">{summary}<time datetime=\"{date_iso}\" \
         data-local-date=\"{first_at}\">{date}</time></p>\
         <ol class=\"share-live-messages\">{messages}</ol>\
         <div class=\"share-actions\"><a class=\"share-button share-button--primary\" \
         href=\"{room}\">{see_room}</a></div>\
         <p class=\"share-disclaimer\">{disclaimer}</p></section>",
        name = escape(&excerpt.room_name),
        date_iso = date_iso(first_at),
        date = escape(&date(first_at, now, language)),
        room = escape(&share_url(origins.site, share_code)),
        see_room = escape(words.see_room),
        disclaimer = escape(render::disclaimer(language)),
    );
    let title = format!("{} · {} · Agent Room", words.excerpt, excerpt.room_name);
    render::shell(language, &title, words.description, origins, &body)
}

/// 读不到第一句了，或者那几句都没了。
pub(super) fn gone_excerpt_page(
    room_name: &str,
    share_code: &str,
    language: Language,
    origins: &Origins<'_>,
) -> String {
    let words = wording(language);
    let body = format!(
        "<section class=\"share-panel\"><p class=\"share-quiet\">{name}</p><h1>{title}</h1>\
         <p class=\"share-quiet\">{text}</p><div class=\"share-actions\">\
         <a class=\"share-button share-button--primary\" href=\"{room}\">{see_room}</a></div>\
         </section>",
        name = escape(room_name),
        title = escape(words.gone_title),
        text = escape(words.gone_text),
        room = escape(&share_url(origins.site, share_code)),
        see_room = escape(words.see_room),
    );
    let title = format!("{} · Agent Room", words.gone_title);
    render::shell(language, &title, words.description, origins, &body)
}

/// 一句：名字、是谁的 Agent、时间，下面是正文。和 `share.js` 里实况的一句一样。
fn message(line: &ExcerptLine, words: &LiveWording) -> String {
    let mut row = format!(
        "<li class=\"share-live-message\"><div class=\"share-live-meta\">\
         <span class=\"share-live-name\">{}</span>",
        escape(&line.name)
    );
    if let Some(kind) = kind(&line.author, words) {
        let _ = write!(
            row,
            "<span class=\"share-live-kind\">{}</span>",
            escape(&kind)
        );
    }
    let _ = write!(
        row,
        "<time class=\"share-live-time\" datetime=\"{iso}\" data-local-time=\"{at}\">{clock}</time>\
         </div>",
        iso = time_iso(line.sent_at_unix_ms),
        at = line.sent_at_unix_ms,
        clock = clock(line.sent_at_unix_ms),
    );
    if line.withheld {
        let _ = write!(
            row,
            "<p class=\"share-live-text share-live-text--quiet\">{}</p></li>",
            escape(words.withheld)
        );
        return row;
    }
    if !line.text.is_empty() {
        let mut text = line.text.clone();
        if line.truncated {
            text.push_str(words.truncated);
        }
        if line.edited {
            text.push_str(words.edited);
        }
        let _ = write!(row, "<p class=\"share-live-text\">{}</p>", escape(&text));
    }
    if line.attachment {
        let _ = write!(
            row,
            "<p class=\"share-live-text share-live-text--quiet\">{}</p>",
            escape(words.file)
        );
    }
    row.push_str("</li>");
    row
}

fn kind(author: &ExcerptAuthor, words: &LiveWording) -> Option<String> {
    match author {
        ExcerptAuthor::Person => None,
        ExcerptAuthor::NetworkAgent => Some(words.network_agent.to_owned()),
        ExcerptAuthor::Agent { owner: Some(owner) } if !owner.is_empty() => {
            Some(words.agent_of.replace("{owner}", owner))
        }
        ExcerptAuthor::Agent { .. } => Some(words.agent.to_owned()),
    }
}

fn count(lines: usize, language: Language) -> String {
    match language {
        Language::Chinese => format!("{lines} 句"),
        Language::English if lines == 1 => "1 message".to_owned(),
        Language::English => format!("{lines} messages"),
    }
}

/// UTC 的那一刻；读不出来（不会发生）时当成 1970 年。
fn utc(unix_ms: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::from_unix_timestamp(unix_ms.div_euclid(1_000))
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
}

/// “11 月 16 日”“Nov 16”；不是今年的带上年份。
fn date(unix_ms: i64, now: UtcMillis, language: Language) -> String {
    let at = utc(unix_ms);
    let month = u8::from(at.month());
    let this_year = at.year() == utc(now.value()).year();
    match (language, this_year) {
        (Language::Chinese, true) => format!("{month} 月 {} 日", at.day()),
        (Language::Chinese, false) => format!("{} 年 {month} 月 {} 日", at.year(), at.day()),
        (Language::English, true) => format!("{} {}", english_month(month), at.day()),
        (Language::English, false) => {
            format!("{} {}, {}", english_month(month), at.day(), at.year())
        }
    }
}

fn english_month(month: u8) -> &'static str {
    ENGLISH_MONTHS
        .get(usize::from(month.saturating_sub(1)))
        .copied()
        .unwrap_or("")
}

fn clock(unix_ms: i64) -> String {
    let at = utc(unix_ms);
    format!("{:02}:{:02}", at.hour(), at.minute())
}

fn date_iso(unix_ms: i64) -> String {
    let at = utc(unix_ms);
    format!(
        "{:04}-{:02}-{:02}",
        at.year(),
        u8::from(at.month()),
        at.day()
    )
}

fn time_iso(unix_ms: i64) -> String {
    let at = utc(unix_ms);
    format!("{}T{:02}:{:02}Z", date_iso(unix_ms), at.hour(), at.minute())
}
