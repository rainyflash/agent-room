//! 把页面交来的一份内容存进“下载”文件夹，再在文件管理器里指给人看。
//!
//! 页面自己的下载（`<a download>`）在桌面端的窗口里靠不住：Windows 上 `WebView2` 悄悄存进“下载”，
//! 窗口里什么也不显示，人以为没反应、连着点好几次；Mac 上没有处理下载的地方，直接被取消，
//! 什么也存不下来。页面要存文件都走这两条命令，存成了回文件名，页面自己写出来。

use std::{
    fs::{self, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::Mutex,
};

use base64::Engine as _;
use serde::Serialize;
use tauri::{AppHandle, Manager as _};
use tauri_plugin_opener::OpenerExt as _;

use crate::commands::DesktopCommandFailure;

/// 一份最多这么大（解码以后）。卡片一两百 KB，“下载我的数据”几 MB 以内。
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;

/// 页面交来的文件名最长多少字节，再长不看。
const MAX_RAW_NAME_BYTES: usize = 512;

/// 文件名（不算扩展名）最多这么多个字符。
const MAX_STEM_CHARACTERS: usize = 80;

/// 只存这几种。页面被人骗着往“下载”里放一个能运行的东西（`.exe`、`.lnk`、`.bat`），
/// 等于替人下了一个程序；要多存一种先想清楚它点开以后会怎样。
const ALLOWED_EXTENSIONS: &[&str] = &["png", "json"];

/// 名字整个不能用时用它。
const FALLBACK_STEM: &str = "agent-room";

/// 重名时在名字后面加 ` (1)`、` (2)`……最多试到这个数。
const MAX_NAME_ATTEMPTS: u32 = 999;

/// 记着这一次运行里最近存的这么多个，“在文件夹里显示”只认它们。
const REMEMBERED_FILES: usize = 32;

/// 这一次运行里存过的文件。页面只拿得到文件名，要指给人看时凭文件名来认：
/// 不让页面交一个路径来，那样等于让它打开这台电脑上的任意位置。
#[derive(Default)]
pub(crate) struct SavedFiles(Mutex<Vec<PathBuf>>);

impl SavedFiles {
    fn remember(&self, path: PathBuf) {
        let mut files = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        files.retain(|known| known != &path);
        if files.len() == REMEMBERED_FILES {
            files.remove(0);
        }
        files.push(path);
    }

    fn find(&self, file_name: &str) -> Option<PathBuf> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .rev()
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy() == file_name)
            })
            .cloned()
    }
}

/// 存成的文件。只回文件名：完整的路径带着这台电脑的用户名，页面用不着。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedFile {
    file_name: String,
}

/// 文件名里系统不让用的、会骗人的字换成空格。
///
/// 控制字符和 `\ / : * ? " < > |` 在 Windows 上不能用；改变文字方向的那几个看不见的字符
/// 能让扩展名看着像别的（`gnp.exe` 那一类），一起去掉。
fn unsafe_in_names(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
        )
        || matches!(
            character,
            '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
        )
}

/// Windows 留给设备的名字。只看第一个点前面那一段：旧一点的 Windows 上 `CON.anything` 也算设备。
fn reserved_device_name(stem: &str) -> bool {
    let first = stem.split('.').next().unwrap_or(stem).trim_end();
    let upper = first.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.ends_with(|last: char| last.is_ascii_digit() && last != '0'))
}

/// 页面交来的文件名能不能用：扩展名要在名单里（不在就不存），名字里不能用的字换掉、太长截短。
/// 回整理好的名字（不带扩展名）和扩展名。
fn checked_name(raw: &str) -> Option<(String, &'static str)> {
    if raw.len() > MAX_RAW_NAME_BYTES {
        return None;
    }
    let (stem, extension) = raw.trim().rsplit_once('.')?;
    let extension = ALLOWED_EXTENSIONS
        .iter()
        .copied()
        .find(|allowed| extension.eq_ignore_ascii_case(allowed))?;
    let cleaned: String = stem
        .chars()
        .map(|character| {
            if unsafe_in_names(character) {
                ' '
            } else {
                character
            }
        })
        .collect();
    let words = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    // 开头的点在 Mac 上是隐藏文件，结尾的点和空格 Windows 会悄悄丢掉。
    let shortened: String = words
        .trim_start_matches('.')
        .chars()
        .take(MAX_STEM_CHARACTERS)
        .collect();
    let stem = shortened.trim_matches(|edge: char| edge == '.' || edge.is_whitespace());
    let stem = if stem.is_empty() {
        FALLBACK_STEM.to_owned()
    } else if reserved_device_name(stem) {
        format!("{FALLBACK_STEM}-{stem}")
    } else {
        stem.to_owned()
    };
    Some((stem, extension))
}

/// 解开页面交来的内容。空的、太大的、不是 base64 的都不存。
fn decoded(content_base64: &str) -> Option<Vec<u8>> {
    // base64 每 4 个字符是 3 个字节，先按长度挡掉太大的，免得白解一遍。
    if content_base64.len() > MAX_FILE_BYTES.div_ceil(3) * 4 {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content_base64)
        .ok()?;
    (!bytes.is_empty() && bytes.len() <= MAX_FILE_BYTES).then_some(bytes)
}

/// 存进这个文件夹。从不盖掉已有的文件：重名就在名字后面加 ` (1)`、` (2)`……
fn save_into(folder: &Path, stem: &str, extension: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(folder)?;
    for attempt in 0..=MAX_NAME_ATTEMPTS {
        let name = if attempt == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem} ({attempt}).{extension}")
        };
        let path = folder.join(name);
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        if let Err(error) = file.write_all(bytes).and_then(|()| file.flush()) {
            drop(file);
            // 写到一半的不留：留着的是一个打不开的文件。
            let _removed = fs::remove_file(&path);
            return Err(error);
        }
        return Ok(path);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "同名的文件太多",
    ))
}

/// 把一份内容存进“下载”文件夹，回存成的文件名（重名时和交来的不一样）。
#[tauri::command(async)]
// Tauri 命令宏按值反序列化参数并提取运行时状态。
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn desktop_save_file(
    app: AppHandle,
    saved: tauri::State<'_, SavedFiles>,
    file_name: String,
    content_base64: String,
) -> Result<SavedFile, DesktopCommandFailure> {
    let (stem, extension) = checked_name(&file_name)
        .ok_or_else(|| DesktopCommandFailure::new("desktop.save.invalid_name", false))?;
    let bytes = decoded(&content_base64)
        .ok_or_else(|| DesktopCommandFailure::new("desktop.save.invalid_content", false))?;
    let folder = app
        .path()
        .download_dir()
        .map_err(|_| DesktopCommandFailure::new("desktop.save.folder_unavailable", false))?;
    let path = save_into(&folder, &stem, extension, &bytes)
        .map_err(|_| DesktopCommandFailure::new("desktop.save.write_failed", true))?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    saved.remember(path);
    Ok(SavedFile { file_name })
}

/// 在文件管理器里指出这一次运行里存过的一个文件；人已经把它挪走、删掉了就打开它原来在的文件夹。
#[tauri::command]
// Tauri 命令宏按值反序列化参数并提取运行时状态。
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn desktop_show_saved_file(
    app: AppHandle,
    saved: tauri::State<'_, SavedFiles>,
    file_name: String,
) -> Result<(), DesktopCommandFailure> {
    let path = saved
        .find(&file_name)
        .ok_or_else(|| DesktopCommandFailure::new("desktop.save.unknown_file", false))?;
    let shown = if path.is_file() {
        app.opener().reveal_item_in_dir(&path)
    } else {
        let folder = path.parent().unwrap_or(&path);
        app.opener()
            .open_path(folder.to_string_lossy(), None::<&str>)
    };
    shown.map_err(|_| DesktopCommandFailure::new("desktop.save.show_failed", true))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use base64::Engine as _;

    use super::{
        MAX_FILE_BYTES, MAX_STEM_CHARACTERS, REMEMBERED_FILES, SavedFiles, checked_name, decoded,
        save_into,
    };

    fn name(raw: &str) -> Option<String> {
        checked_name(raw).map(|(stem, extension)| format!("{stem}.{extension}"))
    }

    #[test]
    fn 平常的名字原样用() {
        assert_eq!(
            name("agent-room-the zeroth dev.png").as_deref(),
            Some("agent-room-the zeroth dev.png")
        );
        assert_eq!(
            name("agent-room-account-2026-10-08.json").as_deref(),
            Some("agent-room-account-2026-10-08.json")
        );
        assert_eq!(
            name("agent-room-项目 房间.PNG").as_deref(),
            Some("agent-room-项目 房间.png")
        );
    }

    #[test]
    fn 只存名单里的几种() {
        for raw in [
            "setup.exe",
            "run.bat",
            "shortcut.lnk",
            "page.html",
            "script.js",
            "card.png.exe",
            "card",
            "",
            ".png.",
        ] {
            assert!(checked_name(raw).is_none(), "{raw}");
        }
    }

    #[test]
    fn 路径和不能用的字换掉() {
        assert_eq!(name("..\\..\\evil.png").as_deref(), Some("evil.png"));
        assert_eq!(
            name("../../etc/passwd.json").as_deref(),
            Some("etc passwd.json")
        );
        assert_eq!(
            name("C:\\Windows\\a.png").as_deref(),
            Some("C Windows a.png")
        );
        assert_eq!(
            name("a<b>c:d\"e|f?g*h.png").as_deref(),
            Some("a b c d e f g h.png")
        );
        assert_eq!(
            name("line\nbreak\ttab.png").as_deref(),
            Some("line break tab.png")
        );
        // 改变文字方向的字符能让扩展名看着像别的。
        assert_eq!(name("card\u{202e}gnp.png").as_deref(), Some("card gnp.png"));
    }

    #[test]
    fn 空的隐藏的和设备名换成能用的() {
        assert_eq!(name(".png").as_deref(), Some("agent-room.png"));
        assert_eq!(name("   .png").as_deref(), Some("agent-room.png"));
        assert_eq!(name("...hidden.png").as_deref(), Some("hidden.png"));
        assert_eq!(name("trailing. .png").as_deref(), Some("trailing.png"));
        assert_eq!(name("CON.png").as_deref(), Some("agent-room-CON.png"));
        assert_eq!(name("com1.json").as_deref(), Some("agent-room-com1.json"));
        assert_eq!(
            name("nul.backup.png").as_deref(),
            Some("agent-room-nul.backup.png")
        );
        assert_eq!(name("COM0.png").as_deref(), Some("COM0.png"));
        assert_eq!(name("console.png").as_deref(), Some("console.png"));
    }

    #[test]
    fn 太长的截短() {
        let long = format!("{}.png", "名".repeat(MAX_STEM_CHARACTERS + 40));
        let (stem, extension) = checked_name(&long).expect("截短以后能用");
        assert_eq!(stem.chars().count(), MAX_STEM_CHARACTERS);
        assert_eq!(extension, "png");
        // 长得离谱的不看。
        assert!(checked_name(&format!("{}.png", "a".repeat(600))).is_none());
    }

    #[test]
    fn 内容要是不空不太大的_base64() {
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        assert_eq!(decoded(&encode(b"card")).as_deref(), Some(&b"card"[..]));
        assert!(decoded("").is_none());
        assert!(decoded("不是 base64").is_none());
        assert!(decoded(&"A".repeat(MAX_FILE_BYTES.div_ceil(3) * 4 + 4)).is_none());
    }

    #[test]
    fn 重名时改名_不盖掉已有的() {
        let folder = tempfile::tempdir().expect("临时目录");
        let first = save_into(folder.path(), "card", "png", b"one").expect("第一份");
        let second = save_into(folder.path(), "card", "png", b"two").expect("第二份");
        let third = save_into(folder.path(), "card", "png", b"three").expect("第三份");
        assert_eq!(first.file_name().unwrap(), "card.png");
        assert_eq!(second.file_name().unwrap(), "card (1).png");
        assert_eq!(third.file_name().unwrap(), "card (2).png");
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        assert_eq!(std::fs::read(&second).unwrap(), b"two");
        assert_eq!(std::fs::read(&third).unwrap(), b"three");
    }

    #[test]
    fn 文件夹不在就建() {
        let root = tempfile::tempdir().expect("临时目录");
        let folder = root.path().join("Downloads");
        let saved = save_into(&folder, "data", "json", b"{}").expect("存成");
        assert_eq!(saved, folder.join("data.json"));
    }

    #[test]
    fn 只认这一次存过的文件名() {
        let saved = SavedFiles::default();
        let folder = PathBuf::from("downloads");
        saved.remember(folder.join("card.png"));
        saved.remember(folder.join("card (1).png"));
        assert_eq!(
            saved.find("card (1).png"),
            Some(folder.join("card (1).png"))
        );
        assert_eq!(saved.find("card.png"), Some(folder.join("card.png")));
        assert_eq!(saved.find("other.png"), None);
        assert_eq!(saved.find("downloads/card.png"), None);
        assert_eq!(saved.find(""), None);
    }

    #[test]
    fn 记不下的忘掉最早的() {
        let saved = SavedFiles::default();
        for index in 0..=REMEMBERED_FILES {
            saved.remember(PathBuf::from(format!("card ({index}).png")));
        }
        assert_eq!(saved.find("card (0).png"), None);
        assert!(saved.find("card (1).png").is_some());
        assert!(
            saved
                .find(&format!("card ({REMEMBERED_FILES}).png"))
                .is_some()
        );
    }
}
