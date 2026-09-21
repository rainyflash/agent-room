//! 读写真实 Windows 凭据库的测试共用的互斥锁和子进程诊断。
//!
//! Windows 凭据管理器不给重叠的 `CredWriteW`、`CredReadW`、`CredDeleteW` 排序：另一个进程
//! 同时读写别的凭据时，本次写入或删除可能返回成功却没有留下来，已删除并确认不存在的凭据
//! 过一会儿又出现。凭据库按用户共享，所以这些测试不只要在同一个测试进程的线程之间，
//! 还要和同一用户的其他测试进程（例如另一个工作区同时跑的测试）一个接一个执行。

use std::{
    fs::{File, OpenOptions},
    process::Output,
};

const LOCK_FILE: &str = "agent-room-test-system-credential-store.lock";

/// 持有期间独占当前用户的系统凭据库，包括持有者启动的子进程；子进程自己不能再取这把锁。
/// 句柄关闭即释放，测试失败展开或进程退出时同样释放。
#[must_use = "丢弃后立即释放系统凭据库"]
pub(crate) struct SystemCredentialStoreLock {
    _file: File,
}

pub(crate) fn lock_system_credential_store() -> SystemCredentialStoreLock {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(std::env::temp_dir().join(LOCK_FILE))
        .expect("可打开系统凭据库测试锁");
    file.lock().expect("可独占系统凭据库测试锁");
    SystemCredentialStoreLock { _file: file }
}

/// libtest 把失败断言写进 stdout，只看 stderr 会得到空白。
pub(crate) fn child_report(output: &Output) -> String {
    format!(
        "{}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
