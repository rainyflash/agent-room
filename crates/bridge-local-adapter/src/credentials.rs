use std::sync::Arc;

use agent_room_bridge_core::ipc::IpcInstallationId;
use agent_room_bridge_ipc::{IpcClientCredentials, IpcSharedSecret};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use keyring::{Entry, Error as KeyringError};

pub const DEFAULT_SECURE_STORAGE_SERVICE: &str = "dev.agent-room.bridge";
pub const IPC_INSTALLATION_ID_ACCOUNT: &str = "bridge-ipc-installation-id-v1";
pub const IPC_SHARED_SECRET_ACCOUNT: &str = "bridge-ipc-shared-secret-v1";
const IPC_SHARED_SECRET_BYTES: usize = 32;

pub trait IpcCredentialSource: Send + Sync {
    /// 读取 Bridge 已经创建的 IPC 安装身份和共享秘密。
    ///
    /// # Errors
    ///
    /// 凭据不存在、OS 安全存储不可用或持久值损坏时返回明确错误。
    fn load(&self) -> Result<IpcClientCredentials, IpcCredentialFailure>;
}

trait CredentialBackend: Send + Sync {
    fn read(&self, account: &str) -> Result<Option<String>, IpcCredentialFailure>;
}

struct KeyringCredentialBackend {
    service: String,
}

impl KeyringCredentialBackend {
    fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

impl CredentialBackend for KeyringCredentialBackend {
    fn read(&self, account: &str) -> Result<Option<String>, IpcCredentialFailure> {
        let entry = Entry::new(&self.service, account).map_err(|_| unavailable())?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(_) => Err(unavailable()),
        }
    }
}

pub struct OsIpcCredentialReader {
    backend: Arc<dyn CredentialBackend>,
}

impl OsIpcCredentialReader {
    pub fn system(service: impl Into<String>) -> Self {
        Self {
            backend: Arc::new(KeyringCredentialBackend::new(service)),
        }
    }

    #[cfg(test)]
    fn new(backend: Arc<dyn CredentialBackend>) -> Self {
        Self { backend }
    }
}

impl IpcCredentialSource for OsIpcCredentialReader {
    fn load(&self) -> Result<IpcClientCredentials, IpcCredentialFailure> {
        let installation_id = self
            .backend
            .read(IPC_INSTALLATION_ID_ACCOUNT)?
            .ok_or_else(missing)?;
        let shared_secret = self
            .backend
            .read(IPC_SHARED_SECRET_ACCOUNT)?
            .ok_or_else(missing)?;
        let installation_id = IpcInstallationId::new(installation_id).map_err(|_| corrupt())?;
        let shared_secret: [u8; IPC_SHARED_SECRET_BYTES] = URL_SAFE_NO_PAD
            .decode(shared_secret)
            .map_err(|_| corrupt())?
            .try_into()
            .map_err(|_| corrupt())?;
        Ok(IpcClientCredentials::new(
            installation_id,
            IpcSharedSecret::new(shared_secret),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcCredentialFailureKind {
    Missing,
    Unavailable,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcCredentialFailure {
    kind: IpcCredentialFailureKind,
}

impl IpcCredentialFailure {
    pub(crate) const fn new(kind: IpcCredentialFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> IpcCredentialFailureKind {
        self.kind
    }
}

const fn missing() -> IpcCredentialFailure {
    IpcCredentialFailure::new(IpcCredentialFailureKind::Missing)
}

const fn unavailable() -> IpcCredentialFailure {
    IpcCredentialFailure::new(IpcCredentialFailureKind::Unavailable)
}

const fn corrupt() -> IpcCredentialFailure {
    IpcCredentialFailure::new(IpcCredentialFailureKind::Corrupt)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    use super::{
        CredentialBackend, IPC_INSTALLATION_ID_ACCOUNT, IPC_SHARED_SECRET_ACCOUNT,
        IpcCredentialFailure, IpcCredentialFailureKind, IpcCredentialSource, OsIpcCredentialReader,
    };

    #[derive(Default)]
    struct 内存凭据(Mutex<HashMap<String, String>>);

    impl CredentialBackend for 内存凭据 {
        fn read(&self, account: &str) -> Result<Option<String>, IpcCredentialFailure> {
            Ok(self.0.lock().expect("测试锁未中毒").get(account).cloned())
        }
    }

    #[test]
    fn 客户端只读取_bridge_已创建的凭据() {
        let backend = Arc::new(内存凭据::default());
        backend.0.lock().expect("测试锁未中毒").extend([
            (
                IPC_INSTALLATION_ID_ACCOUNT.to_owned(),
                "install_1".to_owned(),
            ),
            (
                IPC_SHARED_SECRET_ACCOUNT.to_owned(),
                URL_SAFE_NO_PAD.encode([7_u8; 32]),
            ),
        ]);

        let credentials = OsIpcCredentialReader::new(backend)
            .load()
            .expect("凭据可读取");

        assert_eq!(credentials.installation_id().as_str(), "install_1");
    }

    #[test]
    fn 缺失与损坏凭据绝不会被客户端静默创建() {
        let backend = Arc::new(内存凭据::default());
        let Err(missing) = OsIpcCredentialReader::new(backend.clone()).load() else {
            panic!("缺失凭据必须失败");
        };
        assert_eq!(missing.kind(), IpcCredentialFailureKind::Missing);
        backend.0.lock().expect("测试锁未中毒").extend([
            (
                IPC_INSTALLATION_ID_ACCOUNT.to_owned(),
                "install_1".to_owned(),
            ),
            (IPC_SHARED_SECRET_ACCOUNT.to_owned(), "broken".to_owned()),
        ]);
        let Err(corrupt) = OsIpcCredentialReader::new(backend).load() else {
            panic!("损坏凭据必须失败");
        };
        assert_eq!(corrupt.kind(), IpcCredentialFailureKind::Corrupt);
    }
}
