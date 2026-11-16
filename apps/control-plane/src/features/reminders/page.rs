//! 退订页的 HTML（设计稿“提醒：网页通知与邮件”的退订页）。白底黑字，一页一个黑色的按钮；样式用
//! 分享页那一份。令牌只有字母、数字、`-` 和 `_`，放进页面前照样经 [`escape`]。

use crate::features::share_page::{Language, escape};

struct Wording {
    lang: &'static str,
    confirm_title: &'static str,
    confirm_text: &'static str,
    confirm_button: &'static str,
    later: &'static str,
    settings: &'static str,
    later_after: &'static str,
    done_title: &'static str,
    done_text: &'static str,
    open: &'static str,
    invalid_title: &'static str,
    invalid_text: &'static str,
    unavailable_title: &'static str,
    unavailable_text: &'static str,
    busy_title: &'static str,
    busy_text: &'static str,
}

const CHINESE: Wording = Wording {
    lang: "zh-CN",
    confirm_title: "不再收提醒邮件？",
    confirm_text: "点下面的按钮，就不再发“有人说话你还没看”的提醒邮件。\
                   验证邮箱、重设密码这类账户邮件照常发。",
    confirm_button: "不再发提醒邮件",
    later: "以后想再收，在",
    settings: "设置 › 通用",
    later_after: "里打开。",
    done_title: "已经退订了",
    done_text: "以后不再发提醒邮件。想再收，在设置 › 通用里打开。",
    open: "打开 Agent Room",
    invalid_title: "这个退订链接用不了",
    invalid_text: "链接可能不完整。也可以登录 Agent Room，在设置 › 通用里关掉邮件提醒。",
    unavailable_title: "暂时打不开",
    unavailable_text: "过一会儿再试一次。",
    busy_title: "这一会儿打开得太多了",
    busy_text: "等一分钟再打开。",
};

const ENGLISH: Wording = Wording {
    lang: "en",
    confirm_title: "Stop reminder emails?",
    confirm_text: "Press the button below and we’ll stop sending reminder emails about \
                   messages you haven’t seen. Account emails, like email verification and \
                   password resets, still come.",
    confirm_button: "Stop reminder emails",
    later: "To get them again, turn them on in ",
    settings: "Settings › General",
    later_after: ".",
    done_title: "You’re unsubscribed",
    done_text: "We won’t send you reminder emails anymore. To get them again, turn them on in \
                Settings › General.",
    open: "Open Agent Room",
    invalid_title: "This unsubscribe link doesn’t work",
    invalid_text: "The link may be incomplete. You can also sign in to Agent Room and turn off \
                   email reminders in Settings › General.",
    unavailable_title: "Can’t open this right now",
    unavailable_text: "Try again in a little while.",
    busy_title: "Too many requests right now",
    busy_text: "Wait a minute, then open the link again.",
};

const fn text(language: Language) -> &'static Wording {
    match language {
        Language::Chinese => &CHINESE,
        Language::English => &ENGLISH,
    }
}

/// 还收着：问一句，点了按钮才退。
pub(super) fn confirm(language: Language, token: &str, app: &str) -> String {
    let text = text(language);
    let body = format!(
        "<section class=\"share-panel\"><h1>{title}</h1><p>{message}</p>\
         <form class=\"share-actions\" method=\"post\" action=\"/reminders/unsubscribe\">\
         <input type=\"hidden\" name=\"token\" value=\"{token}\">\
         <button class=\"share-button share-button--primary\" type=\"submit\">{button}</button>\
         </form><p class=\"share-quiet\">{later}<a href=\"{settings_url}\">{settings}</a>\
         {later_after}</p></section>",
        title = escape(text.confirm_title),
        message = escape(text.confirm_text),
        token = escape(token),
        button = escape(text.confirm_button),
        later = escape(text.later),
        settings_url = escape(&settings_url(app)),
        settings = escape(text.settings),
        later_after = escape(text.later_after),
    );
    page(text, text.confirm_title, &body)
}

/// 退了，或者早就退了。
pub(super) fn done(language: Language, app: &str) -> String {
    let text = text(language);
    let body = format!(
        "<section class=\"share-panel\"><h1>{title}</h1><p>{message}</p>\
         <div class=\"share-actions\"><a class=\"share-button\" href=\"{settings_url}\">{open}</a>\
         </div></section>",
        title = escape(text.done_title),
        message = escape(text.done_text),
        settings_url = escape(&settings_url(app)),
        open = escape(text.open),
    );
    page(text, text.done_title, &body)
}

pub(super) fn invalid(language: Language) -> String {
    let text = text(language);
    notice(text, text.invalid_title, text.invalid_text)
}

pub(super) fn unavailable(language: Language) -> String {
    let text = text(language);
    notice(text, text.unavailable_title, text.unavailable_text)
}

pub(super) fn busy(language: Language) -> String {
    let text = text(language);
    notice(text, text.busy_title, text.busy_text)
}

fn notice(text: &Wording, title: &str, message: &str) -> String {
    let body = format!(
        "<section class=\"share-panel\"><h1>{title}</h1><p class=\"share-quiet\">{message}</p>\
         </section>",
        title = escape(title),
        message = escape(message),
    );
    page(text, title, &body)
}

fn settings_url(app: &str) -> String {
    format!("{app}/settings/general")
}

fn page(text: &Wording, title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title} · Agent Room</title>\n<meta name=\"robots\" content=\"noindex\">\n\
         <meta name=\"referrer\" content=\"no-referrer\">\n\
         <link rel=\"stylesheet\" href=\"/r/_/share.css\">\n</head>\n<body>\n\
         <header class=\"share-header\"><span class=\"share-brand\">Agent Room</span></header>\n\
         <main class=\"share-main\">{body}</main>\n</body>\n</html>\n",
        lang = text.lang,
        title = escape(title),
    )
}
