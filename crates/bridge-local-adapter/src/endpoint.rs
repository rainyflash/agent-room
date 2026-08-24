use std::{io, path::Path};

use agent_room_bridge_core::ipc::IpcInstallationId;
use interprocess::local_socket::Name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalIpcEndpoint {
    platform_name: String,
}

impl LocalIpcEndpoint {
    pub fn from_installation(runtime_root: &Path, installation_id: &IpcInstallationId) -> Self {
        #[cfg(windows)]
        let platform_name = {
            let _ = runtime_root;
            format!("agent-room-bridge-{}.sock", installation_id.as_str())
        };
        #[cfg(unix)]
        let platform_name = runtime_root
            .join(format!("bridge-{}.sock", installation_id.as_str()))
            .to_string_lossy()
            .into_owned();
        #[cfg(not(any(windows, unix)))]
        let platform_name = {
            let _ = runtime_root;
            format!("agent-room-bridge-{}.sock", installation_id.as_str())
        };
        Self { platform_name }
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

        self.platform_name.as_str().to_fs_name::<GenericFilePath>()
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
        let endpoint =
            LocalIpcEndpoint::from_installation(Path::new("C:/runtime"), &installation_id);

        assert!(endpoint.to_name().is_ok());
    }
}
