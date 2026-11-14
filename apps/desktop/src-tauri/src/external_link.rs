//! 在系统浏览器里打开页面交来的一个地址。
//!
//! 桌面端的窗口自己打不开新窗口：没有处理“新窗口”请求的地方，页面里的 `target="_blank"` 和
//! `window.open` 都不起作用，opener 插件接管页面链接的开关也关着。页面要打开外面的网页都走
//! 这一条命令。

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt as _;
use url::Url;

use crate::commands::DesktopCommandFailure;

/// 地址最长多少字节。网页端标成能点的地址也是这个上限。
const MAX_LINK_BYTES: usize = 2048;

/// 页面交来的地址能不能交给系统打开。
///
/// 只认 `http`、`https`：别的协议（`file:`、`ms-settings:`、各个应用自己注册的协议）交给系统，
/// 等于让房间里的一条消息在这台电脑上起一个程序。带用户名、密码的也不认：
/// `https://认识的网站@别的网站/` 就是拿来骗人看错域名的。
fn checked_link(raw: &str) -> Option<Url> {
    if raw.len() > MAX_LINK_BYTES {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    if url.host_str().is_none_or(str::is_empty) {
        return None;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    Some(url)
}

/// 在系统浏览器里打开一个 `http`、`https` 地址。
#[tauri::command]
// Tauri 命令宏按值反序列化参数并提取运行时状态。
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn desktop_open_link(app: AppHandle, url: String) -> Result<(), DesktopCommandFailure> {
    let link = checked_link(&url)
        .ok_or_else(|| DesktopCommandFailure::new("desktop.link.invalid", false))?;
    app.opener()
        .open_url(link.as_str(), None::<&str>)
        .map_err(|_| DesktopCommandFailure::new("desktop.link.open_failed", true))
}

#[cfg(test)]
mod tests {
    use super::{MAX_LINK_BYTES, checked_link};

    #[test]
    fn 网页地址原样交给系统() {
        for raw in [
            "https://example.com/",
            "http://example.com:8080/a/b?c=d#e",
            "https://xn--fsqu00a.example/%E4%B8%AD",
            "HTTPS://Example.com/Path",
        ] {
            assert!(checked_link(raw).is_some(), "{raw}");
        }
        assert_eq!(
            checked_link("HTTPS://Example.com/Path").map(String::from),
            Some("https://example.com/Path".to_owned()),
        );
    }

    #[test]
    fn 别的协议一律不接() {
        for raw in [
            "file:///C:/Windows/System32/calc.exe",
            "ms-settings:privacy",
            "agent-room://lobby/abc",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "mailto:someone@example.com",
            "ftp://example.com/file",
            "\\\\server\\share\\run.exe",
            "C:\\Windows\\System32\\calc.exe",
            "example.com",
            "",
        ] {
            assert!(checked_link(raw).is_none(), "{raw}");
        }
    }

    #[test]
    fn 带用户名密码或者没有主机名的不接() {
        for raw in [
            "https://example.com@evil.test/",
            "https://user:secret@example.com/",
            "https://:secret@example.com/",
            "https://",
            "https://?query",
        ] {
            assert!(checked_link(raw).is_none(), "{raw}");
        }
    }

    #[test]
    fn 太长的不接() {
        let prefix = "https://example.com/";
        let fits = format!("{prefix}{}", "a".repeat(MAX_LINK_BYTES - prefix.len()));
        assert!(checked_link(&fits).is_some());
        assert!(checked_link(&format!("{fits}a")).is_none());
    }
}
