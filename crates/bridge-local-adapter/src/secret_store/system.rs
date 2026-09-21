//! 当前用户的系统凭据库：Windows 凭据管理器、macOS 钥匙串或 Secret Service。
//!
//! 工作区只在这里调用 `keyring`。Windows 凭据管理器不给重叠的 `CredWriteW`、`CredReadW`、
//! `CredDeleteW` 排序：与另一个调用重叠时，写入或删除可能返回成功却没有留下来，已删除并确认
//! 不存在的凭据过几秒又出现。只读的调用也会触发，同一进程的不同线程之间同样如此。所以在
//! Windows 上每次调用都独占凭据库：进程内所有线程共用一把锁，同一用户的 Agent Room 进程
//! （桌面、Bridge、CLI、MCP）再通过一个按用户的锁文件一个接一个执行。同时使用凭据管理器的
//! 其他程序不在这把锁之内。

use keyring::{Entry, Error as KeyringError};

use super::SecretStoreFailure;

/// 系统凭据库里一个服务名下的凭据。
#[derive(Debug, Clone)]
pub struct SystemCredentialStore {
    service: String,
}

impl SystemCredentialStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// # Errors
    /// 凭据库不可用时返回错误；凭据不存在时返回 `None`。
    pub fn read(&self, account: &str) -> Result<Option<String>, SecretStoreFailure> {
        match self.with_entry(account, Entry::get_password) {
            Ok(value) => Ok(Some(value)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(_) => Err(SecretStoreFailure::Unavailable),
        }
    }

    /// # Errors
    /// 凭据库拒绝写入时返回错误。
    pub fn write(&self, account: &str, value: &str) -> Result<(), SecretStoreFailure> {
        self.with_entry(account, |entry| entry.set_password(value))
            .map_err(|_| SecretStoreFailure::Unavailable)
    }

    /// # Errors
    /// 凭据库拒绝删除时返回错误；删除不存在的凭据视为成功。
    pub fn delete(&self, account: &str) -> Result<(), SecretStoreFailure> {
        match self.with_entry(account, Entry::delete_credential) {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(_) => Err(SecretStoreFailure::Unavailable),
        }
    }

    /// 条目只在这里创建，所以每次凭据库调用都在独占期间内完成。
    fn with_entry<TValue>(
        &self,
        account: &str,
        operation: impl FnOnce(&Entry) -> keyring::Result<TValue>,
    ) -> keyring::Result<TValue> {
        let _exclusive = exclusive::acquire();
        operation(&Entry::new(&self.service, account)?)
    }
}

#[cfg(not(windows))]
mod exclusive {
    /// 其他平台的凭据库没有发现这个问题，调用方式保持不变。
    pub(super) struct Exclusive;

    pub(super) const fn acquire() -> Exclusive {
        Exclusive
    }
}

#[cfg(windows)]
mod exclusive {
    use std::{
        ffi::OsString,
        fs::{self, File, OpenOptions, TryLockError},
        path::{Path, PathBuf},
        sync::{Mutex, MutexGuard, OnceLock, PoisonError},
        thread,
        time::{Duration, Instant},
    };

    /// 持有者每次只做一次调用，一般不超过百来毫秒；偶尔一次写入要几秒，所以多等一会儿。
    const MAX_WAIT: Duration = Duration::from_secs(5);
    const RETRY_INTERVAL: Duration = Duration::from_millis(5);

    static THREADS: Mutex<()> = Mutex::new(());
    static PROCESSES: OnceLock<Option<File>> = OnceLock::new();

    /// 字段按声明顺序释放：先放开其他进程，再放开本进程的其他线程。
    pub(super) struct Exclusive<'a> {
        _processes: Option<FileLock<'a>>,
        _threads: MutexGuard<'a, ()>,
    }

    pub(super) fn acquire() -> Exclusive<'static> {
        acquire_with(&THREADS, &PROCESSES, MAX_WAIT)
    }

    fn acquire_with<'a>(
        threads: &'a Mutex<()>,
        processes: &'a OnceLock<Option<File>>,
        max_wait: Duration,
    ) -> Exclusive<'a> {
        // 锁里没有数据；持锁线程在凭据库调用中恐慌不会留下半更新的状态。
        let threads = threads.lock().unwrap_or_else(PoisonError::into_inner);
        let processes = processes
            .get_or_init(|| lock_file_path(std::env::var_os).and_then(|path| open(&path)))
            .as_ref()
            .and_then(|file| FileLock::acquire(file, max_wait));
        Exclusive {
            _processes: processes,
            _threads: threads,
        }
    }

    /// 凭据库按用户划分，锁文件也按用户：放在 Bridge 默认数据目录的上一级，不随
    /// `AGENT_ROOM_BRIDGE_DATA_DIR` 改变，同一用户的所有 Agent Room 进程都落到同一个文件。
    fn lock_file_path(read: impl FnOnce(&'static str) -> Option<OsString>) -> Option<PathBuf> {
        let root = PathBuf::from(read("LOCALAPPDATA")?);
        root.is_absolute()
            .then(|| root.join("AgentRoom").join("system-credential-store.lock"))
    }

    /// 打不开就只在进程内排队：锁文件只减少与其他进程重叠，不能让凭据库操作因此失败。
    fn open(path: &Path) -> Option<File> {
        fs::create_dir_all(path.parent()?).ok()?;
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .ok()
    }

    struct FileLock<'a>(&'a File);

    impl<'a> FileLock<'a> {
        /// 等不到就照常调用凭据库，理由同打不开锁文件时。
        fn acquire(file: &'a File, max_wait: Duration) -> Option<Self> {
            let deadline = Instant::now() + max_wait;
            loop {
                match file.try_lock() {
                    Ok(()) => return Some(Self(file)),
                    Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                        thread::sleep(RETRY_INTERVAL);
                    }
                    Err(_) => return None,
                }
            }
        }
    }

    impl Drop for FileLock<'_> {
        fn drop(&mut self) {
            let _ = self.0.unlock();
        }
    }

    #[cfg(test)]
    mod tests {
        use std::{
            ffi::OsString,
            fs::{File, OpenOptions, TryLockError},
            path::Path,
            sync::{
                Mutex, OnceLock,
                atomic::{AtomicUsize, Ordering},
            },
            thread,
            time::{Duration, Instant},
        };

        use super::{FileLock, acquire_with, lock_file_path, open};

        fn handle(path: &Path) -> File {
            OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(path)
                .expect("可打开测试锁文件")
        }

        #[test]
        fn 锁文件按用户放在_bridge_默认数据目录的上一级() {
            let local = std::env::temp_dir();
            assert_eq!(
                lock_file_path(|_| Some(local.clone().into_os_string())),
                Some(local.join("AgentRoom").join("system-credential-store.lock"))
            );
            assert_eq!(lock_file_path(|_| None), None);
            assert_eq!(
                lock_file_path(|_| Some(OsString::from(r"relative\dir"))),
                None
            );
        }

        #[test]
        fn 锁文件在不同句柄之间互斥_释放后别的进程可以接着用() {
            let directory = tempfile::tempdir().expect("可创建临时目录");
            let path = directory.path().join("store.lock");
            let ours = handle(&path);
            // 同一文件的另一个句柄与另一个进程的句柄一样受锁约束。
            let other = handle(&path);
            let locked = FileLock::acquire(&ours, Duration::ZERO).expect("没人持有时立即获得");
            assert!(matches!(other.try_lock(), Err(TryLockError::WouldBlock)));
            drop(locked);
            other.try_lock().expect("释放后可再次获得");
        }

        #[test]
        fn 等不到其他进程释放时不再等待() {
            let directory = tempfile::tempdir().expect("可创建临时目录");
            let path = directory.path().join("store.lock");
            let holder = handle(&path);
            holder.lock().expect("可占住锁文件");
            let started = Instant::now();
            assert!(FileLock::acquire(&handle(&path), Duration::from_millis(50)).is_none());
            assert!(started.elapsed() >= Duration::from_millis(50));
        }

        #[test]
        fn 同一进程的线程一个接一个进入() {
            let directory = tempfile::tempdir().expect("可创建临时目录");
            let threads = Mutex::new(());
            let processes = OnceLock::from(Some(handle(&directory.path().join("store.lock"))));
            let inside = AtomicUsize::new(0);
            let overlaps = AtomicUsize::new(0);
            thread::scope(|scope| {
                for _ in 0..8 {
                    scope.spawn(|| {
                        for _ in 0..50 {
                            let _exclusive =
                                acquire_with(&threads, &processes, Duration::from_secs(5));
                            if inside.fetch_add(1, Ordering::SeqCst) != 0 {
                                overlaps.fetch_add(1, Ordering::SeqCst);
                            }
                            thread::yield_now();
                            inside.fetch_sub(1, Ordering::SeqCst);
                        }
                    });
                }
            });
            assert_eq!(overlaps.load(Ordering::SeqCst), 0);
            // 最后一个持有者退出后，锁文件也已放开。
            handle(&directory.path().join("store.lock"))
                .try_lock()
                .expect("全部释放后可获得锁文件");
        }

        #[test]
        fn 打不开锁文件时只在进程内排队() {
            assert!(open(&std::env::temp_dir().join("\0")).is_none());
            let threads = Mutex::new(());
            let processes = OnceLock::from(None);
            let _exclusive = acquire_with(&threads, &processes, Duration::ZERO);
            assert!(threads.try_lock().is_err());
        }
    }
}
