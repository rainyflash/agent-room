use std::{
    fs::{self, File, OpenOptions},
    io::{Seek as _, SeekFrom, Write as _},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};

#[cfg(unix)]
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BridgeRuntimePaths {
    data_root: PathBuf,
    runtime_root: PathBuf,
    matrix_store_root: PathBuf,
    handoff_root: PathBuf,
    handoff_database: PathBuf,
    instance_lock: PathBuf,
    matrix_store_lock: PathBuf,
}

impl BridgeRuntimePaths {
    pub(crate) fn new(data_root: PathBuf) -> Self {
        let handoff_root = data_root.join("handoffs");
        Self {
            runtime_root: data_root.join("runtime"),
            matrix_store_root: data_root.join("matrix-store"),
            handoff_database: handoff_root.join("handoffs.sqlite"),
            handoff_root,
            instance_lock: data_root.join("bridge.lock"),
            matrix_store_lock: data_root.join("matrix-store.lock"),
            data_root,
        }
    }

    /// 创建并验证 Bridge 私有运行目录。
    ///
    /// # Errors
    ///
    /// 目录不可创建、不是目录或 Unix 权限宽于当前用户时返回错误。
    pub(crate) fn prepare(&self) -> BridgeRuntimeFileResult<()> {
        create_private_directory(&self.data_root)?;
        create_private_directory(&self.runtime_root)?;
        create_private_directory(&self.matrix_store_root)?;
        create_private_directory(&self.handoff_root)
    }

    pub(crate) fn instance_lock_path(&self) -> &Path {
        &self.instance_lock
    }

    pub(crate) fn matrix_store_lock_path(&self) -> &Path {
        &self.matrix_store_lock
    }

    pub(crate) fn matrix_store_root(&self) -> &Path {
        &self.matrix_store_root
    }

    pub(crate) fn handoff_database(&self) -> &Path {
        &self.handoff_database
    }

    #[cfg(unix)]
    pub(crate) fn runtime_root(&self) -> &Path {
        &self.runtime_root
    }
}

#[derive(Debug)]
pub(crate) struct BridgeExclusiveLock {
    _file: File,
}

impl BridgeExclusiveLock {
    /// 非阻塞获取跨进程独占锁，并在成功后写入诊断 PID。
    ///
    /// # Errors
    ///
    /// 锁已由其他进程持有、路径不安全或文件 I/O 失败时返回稳定错误。
    pub(crate) fn acquire(path: &Path) -> BridgeRuntimeFileResult<Self> {
        let parent = path.parent().ok_or_else(|| {
            BridgeRuntimeFileFailure::new(BridgeRuntimeFileFailureKind::InvalidPath)
        })?;
        create_private_directory(parent)?;
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        options.mode(PRIVATE_FILE_MODE);
        let mut file = options.open(path).map_err(map_io_failure)?;
        #[cfg(unix)]
        validate_private_file(&file)?;
        file.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => {
                BridgeRuntimeFileFailure::new(BridgeRuntimeFileFailureKind::AlreadyHeld)
            }
            fs::TryLockError::Error(error) => map_io_failure(error),
        })?;
        file.set_len(0).map_err(map_io_failure)?;
        file.seek(SeekFrom::Start(0)).map_err(map_io_failure)?;
        writeln!(file, "pid={}", std::process::id()).map_err(map_io_failure)?;
        file.flush().map_err(map_io_failure)?;
        Ok(Self { _file: file })
    }
}

fn create_private_directory(path: &Path) -> BridgeRuntimeFileResult<()> {
    if !path.exists() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(PRIVATE_DIRECTORY_MODE);
        builder.create(path).map_err(map_io_failure)?;
    }
    let metadata = fs::metadata(path).map_err(map_io_failure)?;
    if !metadata.is_dir() {
        return Err(BridgeRuntimeFileFailure::new(
            BridgeRuntimeFileFailureKind::InvalidPath,
        ));
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(BridgeRuntimeFileFailure::new(
            BridgeRuntimeFileFailureKind::InsecurePermissions,
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_private_file(file: &File) -> BridgeRuntimeFileResult<()> {
    if file
        .metadata()
        .map_err(map_io_failure)?
        .permissions()
        .mode()
        & 0o077
        != 0
    {
        return Err(BridgeRuntimeFileFailure::new(
            BridgeRuntimeFileFailureKind::InsecurePermissions,
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BridgeRuntimeFileFailureKind {
    InvalidPath,
    #[cfg(unix)]
    InsecurePermissions,
    AlreadyHeld,
    Io,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BridgeRuntimeFileFailure {
    kind: BridgeRuntimeFileFailureKind,
}

impl BridgeRuntimeFileFailure {
    const fn new(kind: BridgeRuntimeFileFailureKind) -> Self {
        Self { kind }
    }

    pub(crate) const fn kind(self) -> BridgeRuntimeFileFailureKind {
        self.kind
    }
}

pub(crate) type BridgeRuntimeFileResult<T> = Result<T, BridgeRuntimeFileFailure>;

fn map_io_failure(_error: std::io::Error) -> BridgeRuntimeFileFailure {
    BridgeRuntimeFileFailure::new(BridgeRuntimeFileFailureKind::Io)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    use tempfile::tempdir;

    use super::{BridgeExclusiveLock, BridgeRuntimeFileFailureKind, BridgeRuntimePaths};

    #[test]
    fn 同一进程内重复锁不能依赖_pid_文本抢占() {
        let temporary = tempdir().expect("测试目录可创建");
        let paths = BridgeRuntimePaths::new(temporary.path().join("bridge"));
        paths.prepare().expect("运行目录可准备");
        let first =
            BridgeExclusiveLock::acquire(paths.instance_lock_path()).expect("首个进程可获取锁");

        let failure = BridgeExclusiveLock::acquire(paths.instance_lock_path())
            .expect_err("第二个进程锁必须失败");

        assert_eq!(failure.kind(), BridgeRuntimeFileFailureKind::AlreadyHeld);
        drop(first);
        BridgeExclusiveLock::acquire(paths.instance_lock_path()).expect("释放后可再次获取锁");
    }

    #[test]
    fn 另一进程不能获取守护进程锁() {
        let temporary = tempdir().expect("测试目录可创建");
        let paths = BridgeRuntimePaths::new(temporary.path().join("bridge"));
        paths.prepare().expect("运行目录可准备");

        assert_other_process_cannot_acquire(paths.instance_lock_path(), temporary.path());
    }

    #[test]
    fn 另一进程不能获取矩阵存储锁() {
        let temporary = tempdir().expect("测试目录可创建");
        let paths = BridgeRuntimePaths::new(temporary.path().join("bridge"));
        paths.prepare().expect("运行目录可准备");

        assert_other_process_cannot_acquire(paths.matrix_store_lock_path(), temporary.path());
    }

    #[test]
    fn 子进程持锁助手() {
        let Ok(lock_path) = std::env::var("AGENT_ROOM_TEST_LOCK_HELPER_PATH") else {
            return;
        };
        let ready_path = PathBuf::from(
            std::env::var("AGENT_ROOM_TEST_LOCK_HELPER_READY").expect("助手必须收到就绪路径"),
        );
        let _lock = BridgeExclusiveLock::acquire(Path::new(&lock_path)).expect("子进程应获取锁");
        fs::write(&ready_path, b"ready").expect("子进程应写入就绪标记");
        thread::sleep(Duration::from_secs(30));
    }

    fn assert_other_process_cannot_acquire(lock_path: &Path, temporary_root: &Path) {
        let ready_path = temporary_root.join("child-ready");
        let child = Command::new(std::env::current_exe().expect("应找到当前测试程序"))
            .args([
                "--exact",
                "runtime_files::tests::子进程持锁助手",
                "--nocapture",
            ])
            .env("AGENT_ROOM_TEST_LOCK_HELPER_PATH", lock_path)
            .env("AGENT_ROOM_TEST_LOCK_HELPER_READY", &ready_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("应启动持锁子进程");
        let mut child = ChildGuard(child);
        wait_until_ready(&mut child.0, &ready_path);

        let failure = BridgeExclusiveLock::acquire(lock_path).expect_err("另一进程持锁时必须失败");

        assert_eq!(failure.kind(), BridgeRuntimeFileFailureKind::AlreadyHeld);
    }

    fn wait_until_ready(child: &mut Child, ready_path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready_path.is_file() {
            assert!(Instant::now() < deadline, "子进程未在时限内持有锁");
            assert!(
                child.try_wait().expect("应读取子进程状态").is_none(),
                "子进程在持锁前异常退出"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }

    #[test]
    fn 矩阵存储锁与守护进程锁是两个明确边界() {
        let temporary = tempdir().expect("测试目录可创建");
        let paths = BridgeRuntimePaths::new(temporary.path().join("bridge"));
        paths.prepare().expect("运行目录可准备");
        let _instance =
            BridgeExclusiveLock::acquire(paths.instance_lock_path()).expect("守护进程锁可获取");
        let store =
            BridgeExclusiveLock::acquire(paths.matrix_store_lock_path()).expect("存储锁可独立获取");

        assert_eq!(
            BridgeExclusiveLock::acquire(paths.matrix_store_lock_path())
                .expect_err("重复存储锁必须失败")
                .kind(),
            BridgeRuntimeFileFailureKind::AlreadyHeld
        );
        drop(store);
        assert!(paths.runtime_root.is_dir());
        assert!(paths.matrix_store_root.is_dir());
        assert!(paths.handoff_root.is_dir());
        assert_eq!(
            paths.handoff_database(),
            paths.handoff_root.join("handoffs.sqlite")
        );
    }

    #[cfg(unix)]
    #[test]
    fn 拒绝沿用权限过宽的运行目录() {
        use std::{fs, os::unix::fs::PermissionsExt as _};

        let temporary = tempdir().expect("测试目录可创建");
        let root = temporary.path().join("bridge");
        fs::create_dir(&root).expect("测试目录可创建");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).expect("测试权限可设置");
        let paths = BridgeRuntimePaths::new(root);

        assert_eq!(
            paths.prepare().expect_err("宽权限目录必须失败").kind(),
            BridgeRuntimeFileFailureKind::InsecurePermissions
        );
    }
}
