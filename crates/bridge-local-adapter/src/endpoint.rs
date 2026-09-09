use std::{io, path::Path};

#[cfg(unix)]
use std::path::PathBuf;

use agent_room_bridge_core::ipc::IpcInstallationId;
use interprocess::local_socket::Name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalIpcEndpoint {
    #[cfg(not(unix))]
    platform_name: String,
    #[cfg(unix)]
    platform_path: PathBuf,
}

impl LocalIpcEndpoint {
    pub fn from_installation(runtime_root: &Path, installation_id: &IpcInstallationId) -> Self {
        #[cfg(windows)]
        let endpoint = {
            let _ = runtime_root;
            Self {
                platform_name: format!("agent-room-bridge-{}.sock", installation_id.as_str()),
            }
        };
        #[cfg(unix)]
        let endpoint = {
            let _ = installation_id;
            Self {
                platform_path: runtime_root.join("bridge.sock"),
            }
        };
        #[cfg(not(any(windows, unix)))]
        let endpoint = {
            let _ = runtime_root;
            Self {
                platform_name: format!("agent-room-bridge-{}.sock", installation_id.as_str()),
            }
        };
        endpoint
    }

    /// Recover a socket left behind by an interrupted Bridge process.
    /// The caller must hold the installation's exclusive lock in its private runtime directory.
    /// Active listeners, regular files and symbolic links are never removed.
    ///
    /// # Errors
    /// Unknown ownership, a live listener or an unsafe/replaced path prevents recovery.
    #[cfg(unix)]
    pub async fn reclaim_stale_socket(&self) -> io::Result<()> {
        use std::fs;
        use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
        use std::time::Duration;

        let path = &self.platform_path;
        let previous = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        if !previous.file_type().is_socket() || previous.uid() != fs::metadata(parent)?.uid() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "IPC path is not an owned socket",
            ));
        }
        match tokio::time::timeout(
            Duration::from_millis(250),
            tokio::net::UnixStream::connect(path),
        )
        .await
        {
            Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => {}
            Ok(Err(error)) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Ok(Err(error)) => return Err(error),
            Ok(Ok(_)) | Err(_) => return Err(io::Error::from(io::ErrorKind::AddrInUse)),
        }
        let current = fs::symlink_metadata(path)?;
        if current.dev() != previous.dev()
            || current.ino() != previous.ino()
            || !current.file_type().is_socket()
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "IPC path changed during recovery",
            ));
        }
        fs::remove_file(path)
    }

    /// 转换为当前操作系统的命名管道或 Unix Socket 名称。
    ///
    /// # Errors
    ///
    /// 平台命名规则拒绝端点时返回 I/O 错误。
    #[cfg(windows)]
    pub fn to_name(&self) -> io::Result<Name<'_>> {
        use interprocess::local_socket::{GenericNamespaced, ToNsName as _};

        self.platform_name
            .as_str()
            .to_ns_name::<GenericNamespaced>()
    }

    /// 转换为当前操作系统的命名管道或 Unix Socket 名称。
    ///
    /// # Errors
    ///
    /// 平台命名规则拒绝端点时返回 I/O 错误。
    #[cfg(unix)]
    pub fn to_name(&self) -> io::Result<Name<'_>> {
        use interprocess::local_socket::{GenericFilePath, ToFsName as _};
        use std::os::unix::ffi::OsStrExt as _;

        const MAX_PORTABLE_SOCKET_PATH_BYTES: usize = 103;
        let bytes = self.platform_path.as_os_str().as_bytes();
        if bytes.len() > MAX_PORTABLE_SOCKET_PATH_BYTES || bytes.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Unix Socket 路径超过可移植上限",
            ));
        }
        self.platform_path.as_path().to_fs_name::<GenericFilePath>()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use agent_room_bridge_core::ipc::IpcInstallationId;

    use super::LocalIpcEndpoint;

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_崩溃遗留端点可回收并再次监听() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = LocalIpcEndpoint::from_installation(
            directory.path(),
            &IpcInstallationId::new("test_stale").unwrap(),
        );
        let listener = std::os::unix::net::UnixListener::bind(&endpoint.platform_path).unwrap();
        drop(listener); // std listener leaves the socket file, just like an interrupted process.
        assert!(endpoint.platform_path.exists());
        endpoint.reclaim_stale_socket().await.unwrap();
        assert!(!endpoint.platform_path.exists());
        let _rebound = std::os::unix::net::UnixListener::bind(&endpoint.platform_path).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_活动监听器不能被回收或抢占() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = LocalIpcEndpoint::from_installation(
            directory.path(),
            &IpcInstallationId::new("test_active").unwrap(),
        );
        let _listener = std::os::unix::net::UnixListener::bind(&endpoint.platform_path).unwrap();
        assert_eq!(
            endpoint.reclaim_stale_socket().await.unwrap_err().kind(),
            std::io::ErrorKind::AddrInUse
        );
        assert!(endpoint.platform_path.exists());
        let _client = tokio::net::UnixStream::connect(&endpoint.platform_path)
            .await
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_回收拒绝删除普通文件和符号链接() {
        for symbolic in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let endpoint = LocalIpcEndpoint::from_installation(
                directory.path(),
                &IpcInstallationId::new("test_file").unwrap(),
            );
            let original = directory.path().join("original");
            std::fs::write(&original, b"preserved").unwrap();
            if symbolic {
                std::os::unix::fs::symlink(&original, &endpoint.platform_path).unwrap();
            } else {
                std::fs::write(&endpoint.platform_path, b"preserved").unwrap();
            }
            assert!(endpoint.reclaim_stale_socket().await.is_err());
            assert_eq!(
                std::fs::read(&endpoint.platform_path).unwrap(),
                b"preserved"
            );
            assert_eq!(std::fs::read(&original).unwrap(), b"preserved");
            assert_eq!(
                std::fs::symlink_metadata(&endpoint.platform_path)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                symbolic
            );
        }
    }

    #[test]
    fn 安装标识稳定映射到当前平台端点() {
        let installation_id = IpcInstallationId::new("install_1").expect("安装标识有效");
        let runtime_root = if cfg!(unix) {
            Path::new("/runtime")
        } else {
            Path::new("C:/runtime")
        };
        let endpoint = LocalIpcEndpoint::from_installation(runtime_root, &installation_id);

        assert!(endpoint.to_name().is_ok());
        #[cfg(unix)]
        {
            assert_eq!(endpoint.platform_path, Path::new("/runtime/bridge.sock"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_端点适配持续集成的深层运行目录() {
        let installation_id = IpcInstallationId::new("install_1").expect("安装标识有效");
        let runtime_root = Path::new(
            "/home/runner/work/agent-room/agent-room/.local/vertical/bridge-sender/runtime",
        );
        let endpoint = LocalIpcEndpoint::from_installation(runtime_root, &installation_id);

        assert_eq!(endpoint.platform_path, runtime_root.join("bridge.sock"));
        assert!(endpoint.to_name().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn unix_端点在进入内核前拒绝过长路径() {
        let installation_id = IpcInstallationId::new("install_1").expect("安装标识有效");
        let runtime_root = Path::new("/").join("x".repeat(104));
        let endpoint = LocalIpcEndpoint::from_installation(&runtime_root, &installation_id);

        let failure = endpoint.to_name().expect_err("过长路径必须被拒绝");

        assert_eq!(failure.kind(), std::io::ErrorKind::InvalidInput);
    }
}
