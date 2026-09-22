//! 桌面端和 Bridge 各写一份本机日志文件，出了问题用户有东西可发。
//!
//! 只有两代：写满上限后把当前文件改名为 `.1`（覆盖上一代），再从头写。
//! 日志内容由调用方负责：只记状态、错误码和数量，不记消息内容和凭据。

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// 每个进程的日志目录：`<数据根>/logs`。
#[must_use]
pub fn bridge_log_root(data_root: &Path) -> PathBuf {
    data_root.join("logs")
}

/// 单个日志文件的默认上限；两代合计不超过它的两倍。
pub const DEFAULT_LOG_FILE_CAP_BYTES: u64 = 5 * 1024 * 1024;

/// 可克隆的追加写入器，`tracing_subscriber` 的 `with_writer(move || file.clone())` 可以直接用。
#[derive(Clone)]
pub struct RotatingLogFile {
    state: Arc<Mutex<State>>,
}

struct State {
    path: PathBuf,
    file: File,
    written: u64,
    cap: u64,
}

impl RotatingLogFile {
    /// 打开（或创建）日志文件并接着上次的末尾写。
    ///
    /// # Errors
    ///
    /// 目录建不出来或文件打不开时返回错误；调用方通常改为只写 stderr。
    pub fn open(path: PathBuf, cap: u64) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            create_private_directories(parent)?;
        }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        let written = file.metadata()?.len();
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                path,
                file,
                written,
                cap: cap.max(1),
            })),
        })
    }

    /// 日志文件的路径。
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.state.lock().map_or_else(
            |poisoned| poisoned.into_inner().path.clone(),
            |state| state.path.clone(),
        )
    }
}

impl State {
    fn rotate(&mut self) -> io::Result<()> {
        let previous = previous_generation(&self.path);
        let _ = std::fs::remove_file(&previous);
        std::fs::rename(&self.path, &previous)?;
        self.file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.path)?;
        self.written = 0;
        Ok(())
    }
}

/// 与 Bridge 的运行目录同样只对当前用户开放：数据根可能由日志先建出来，Bridge 随后会检查它的权限。
fn create_private_directories(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn previous_generation(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map_or_else(Default::default, std::ffi::OsStr::to_os_string);
    name.push(".1");
    path.with_file_name(name)
}

impl Write for RotatingLogFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // 只在一条记录的边界上换代，记录本身不拆开。
        if state.written > 0 && state.written.saturating_add(buf.len() as u64) > state.cap {
            state.rotate()?;
        }
        state.file.write_all(buf)?;
        state.written = state.written.saturating_add(buf.len() as u64);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .file
            .flush()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::{RotatingLogFile, bridge_log_root};

    #[test]
    fn 写满上限后换代且旧代只保留一份() {
        let directory = tempfile::tempdir().expect("临时目录");
        let path = directory.path().join("logs").join("bridge.log");
        let mut log = RotatingLogFile::open(path.clone(), 16).expect("打开日志");
        log.write_all(b"first record\n").expect("写入");
        log.write_all(b"second record\n").expect("写入触发换代");
        log.write_all(b"third record\n").expect("再次换代");

        assert_eq!(
            std::fs::read_to_string(&path).expect("当前代"),
            "third record\n"
        );
        assert_eq!(
            std::fs::read_to_string(path.with_file_name("bridge.log.1")).expect("上一代"),
            "second record\n"
        );
        assert!(!path.with_file_name("bridge.log.2").exists());
    }

    #[test]
    fn 重新打开接着上次末尾写并按已有长度计数() {
        let directory = tempfile::tempdir().expect("临时目录");
        let path = directory.path().join("desktop.log");
        RotatingLogFile::open(path.clone(), 64)
            .expect("打开")
            .write_all(b"kept\n")
            .expect("写入");
        let mut reopened = RotatingLogFile::open(path.clone(), 64).expect("再次打开");
        reopened.write_all(b"appended\n").expect("写入");
        assert_eq!(
            std::fs::read_to_string(&path).expect("读取"),
            "kept\nappended\n"
        );
        assert_eq!(reopened.path(), path);
    }

    #[cfg(unix)]
    #[test]
    fn 日志目录和文件只对当前用户开放() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("临时目录");
        let root = directory.path().join("data");
        let path = root.join("logs").join("bridge.log");
        RotatingLogFile::open(path.clone(), 64).expect("打开");
        for created in [&root, &root.join("logs")] {
            assert_eq!(
                std::fs::metadata(created)
                    .expect("目录")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        assert_eq!(
            std::fs::metadata(&path).expect("文件").permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn 日志目录在数据根之下() {
        let root = std::path::Path::new("data");
        assert_eq!(bridge_log_root(root), root.join("logs"));
    }
}
