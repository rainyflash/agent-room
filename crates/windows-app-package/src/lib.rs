//! 在打包应用（MSIX）的包外面启动程序。
//!
//! 从商店装的打包应用（比如某些 Agent 宿主）起的子进程，默认都算在那个应用的包里。Windows 给包里的
//! 进程看的 `AppData` 是真实目录和这个应用私有副本（`%LOCALAPPDATA%\Packages\<包>\LocalCache`）
//! 合在一起的样子：新建的文件写进私有副本，改已有的文件改的是真实目录。正常启动的程序看不到私有副本。
//!
//! Agent 连不上 Bridge 时会在后台拉起桌面端（#221）。宿主是打包应用时，桌面端、它托管的 Bridge 和
//! `WebView` 就都跑在宿主的包里，本机数据一半写进宿主的私有副本：下次正常启动时数据库对不上，
//! 换服务器时收走的旧设置也还留在副本里被读到。所以拉起桌面端要用这里的函数，让它在包外运行。
//!
//! 实测（2026-10-24，Windows 11）：创建子进程时带上
//! `PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY = PROCESS_CREATION_DESKTOP_APP_BREAKAWAY_ENABLE_PROCESS_TREE`，
//! 子进程本身就在包外，它再起的进程也在包外；不带或带 `..._OVERRIDE` 都留在包里。

#[cfg(any(windows, test))]
mod command_line;
#[cfg(windows)]
mod ffi;

#[cfg(windows)]
pub use windows::{inside_app_package, spawn_outside_app_package};

#[cfg(windows)]
mod windows {
    use std::{ffi::OsString, io, os::windows::ffi::OsStrExt as _, path::Path};

    use crate::{command_line, ffi};

    /// 当前进程是不是在某个打包应用的包里运行（继承来的也算）。
    pub fn inside_app_package() -> bool {
        ffi::inside_app_package()
    }

    /// 在任何打包应用的包外面启动 `program`，返回进程号，不等它结束。
    ///
    /// `flags` 是 `CreateProcessW` 的创建标志（比如 `DETACHED_PROCESS`）。子进程不继承任何句柄，
    /// 标准输入输出都是空的；环境和工作目录照当前进程。
    ///
    /// # Errors
    ///
    /// 程序路径里有双引号、带了 NUL，或者系统拒绝创建进程（比如 `CREATE_BREAKAWAY_FROM_JOB`
    /// 被所在的作业禁止）时返回错误。
    pub fn spawn_outside_app_package(
        program: &Path,
        arguments: &[OsString],
        flags: u32,
    ) -> io::Result<u32> {
        let application = wide_with_nul(program.as_os_str().encode_wide())?;
        let mut line = Vec::new();
        command_line::append_program(&mut line, &application[..application.len() - 1])?;
        for argument in arguments {
            line.push(u16::from(b' '));
            command_line::append_argument(&mut line, &wide(argument.encode_wide())?);
        }
        line.push(0);
        ffi::create_process_outside_package(&application, &mut line, flags)
    }

    fn wide(units: impl Iterator<Item = u16>) -> io::Result<Vec<u16>> {
        let units: Vec<u16> = units.collect();
        if units.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "command line contains NUL",
            ));
        }
        Ok(units)
    }

    fn wide_with_nul(units: impl Iterator<Item = u16>) -> io::Result<Vec<u16>> {
        let mut units = wide(units)?;
        units.push(0);
        Ok(units)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::{ffi::OsString, path::PathBuf, time::Duration};

    use super::{inside_app_package, spawn_outside_app_package};

    const DETACHED_PROCESS: u32 = 0x0000_0008;

    #[test]
    fn 测试进程不在打包应用里() {
        assert!(!inside_app_package());
    }

    #[test]
    fn 在包外启动的程序收到原样的参数() {
        let directory = tempfile::tempdir().expect("临时目录可建");
        let marker = directory.path().join("带 空格 的标记");
        let shell = PathBuf::from(std::env::var_os("ComSpec").expect("Windows 有 ComSpec"));
        let arguments = ["/d", "/c", "copy", "nul"]
            .into_iter()
            .map(OsString::from)
            .chain([marker.clone().into_os_string()])
            .collect::<Vec<_>>();

        spawn_outside_app_package(&shell, &arguments, DETACHED_PROCESS).expect("能启动");

        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(marker.exists(), "子进程按原样拿到了带空格的路径");
    }

    #[test]
    fn 程序路径不许带双引号() {
        let error = spawn_outside_app_package(
            &PathBuf::from(r#"C:\"quoted"\app.exe"#),
            &[],
            DETACHED_PROCESS,
        )
        .expect_err("双引号会让命令行解析错位");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
