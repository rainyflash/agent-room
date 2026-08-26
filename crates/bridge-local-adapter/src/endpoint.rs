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
