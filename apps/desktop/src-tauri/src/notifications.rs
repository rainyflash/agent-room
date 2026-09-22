//! 网页层判断出「有人在找你、窗口又不在前台」后，由这里发一条系统通知。
//! 通知只带发件人和一小段正文，长度在这里再兜一次底，控制字符去掉。

use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt as _;

use crate::commands::DesktopCommandFailure;

const TITLE_LIMIT: usize = 80;
const BODY_LIMIT: usize = 240;

#[tauri::command]
// Tauri 命令宏按值反序列化参数并提取运行时状态。
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn desktop_notify(
    app: AppHandle,
    title: String,
    body: String,
) -> Result<(), DesktopCommandFailure> {
    let title = clean(&title, TITLE_LIMIT);
    let body = clean(&body, BODY_LIMIT);
    if title.is_empty() && body.is_empty() {
        return Err(DesktopCommandFailure::new(
            "desktop.notification.empty",
            false,
        ));
    }
    app.notification()
        .builder()
        .title(if title.is_empty() {
            "Agent Room"
        } else {
            &title
        })
        .body(body)
        .show()
        .map_err(|_| DesktopCommandFailure::new("desktop.notification.failed", true))
}

/// 去掉控制字符、折叠空白并按字符数截断；截断后补省略号。
fn clean(value: &str, limit: usize) -> String {
    let collapsed = value
        .split(char::is_whitespace)
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut kept: String = collapsed
        .chars()
        .filter(|character| !character.is_control())
        .take(limit)
        .collect();
    if collapsed.chars().filter(|c| !c.is_control()).count() > limit {
        kept.pop();
        kept.push('…');
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::{BODY_LIMIT, clean};

    #[test]
    fn 通知文本去控制字符折叠空白并截断() {
        assert_eq!(
            clean("  astra:\tcan you\n\nreview?\u{7} ", 80),
            "astra: can you review?"
        );
        let long = clean(&"字".repeat(BODY_LIMIT + 5), BODY_LIMIT);
        assert_eq!(long.chars().count(), BODY_LIMIT);
        assert!(long.ends_with('…'));
        assert_eq!(clean("exact", 5), "exact");
    }
}
