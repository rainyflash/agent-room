use std::sync::{Arc, Mutex};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{desktop_config::DesktopBridgeConfig, matrix_session::MatrixSessionFailure};

const ACCOUNT: &str = "matrix-human-session-v1";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredMatrixSession {
    access_token: String,
    device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    user_id: String,
    version: u8,
}

impl StoredMatrixSession {
    fn validate(&self) -> Result<(), MatrixSessionFailure> {
        let valid_user = self
            .user_id
            .strip_prefix('@')
            .and_then(|value| value.split_once(':'))
            .is_some_and(|(localpart, server)| !localpart.is_empty() && !server.is_empty());
        if self.version != 1
            || !valid_user
            || !valid_value(&self.user_id, 255)
            || !valid_value(&self.device_id, 255)
            || !valid_value(&self.access_token, 4_096)
            || self
                .refresh_token
                .as_ref()
                .is_some_and(|value| !valid_value(value, 4_096))
        {
            return Err(MatrixSessionFailure::new(
                "desktop.matrix_session.credentials_invalid",
                false,
            ));
        }
        Ok(())
    }
}

trait MatrixCredentialVault: Send + Sync {
    fn load(&self) -> Result<Option<String>, MatrixSessionFailure>;
    fn save(&self, serialized: &str) -> Result<(), MatrixSessionFailure>;
    fn clear(&self) -> Result<(), MatrixSessionFailure>;
}

struct KeyringMatrixCredentialVault {
    service: String,
}

impl MatrixCredentialVault for KeyringMatrixCredentialVault {
    fn load(&self) -> Result<Option<String>, MatrixSessionFailure> {
        match Entry::new(&self.service, ACCOUNT).and_then(|entry| entry.get_password()) {
            Ok(value) => Ok(Some(value)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(_) => Err(unavailable()),
        }
    }

    fn save(&self, serialized: &str) -> Result<(), MatrixSessionFailure> {
        Entry::new(&self.service, ACCOUNT)
            .and_then(|entry| entry.set_password(serialized))
            .map_err(|_| unavailable())
    }

    fn clear(&self) -> Result<(), MatrixSessionFailure> {
        match Entry::new(&self.service, ACCOUNT).and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(_) => Err(unavailable()),
        }
    }
}

pub(crate) struct MatrixCredentialRuntime {
    vault: Arc<dyn MatrixCredentialVault>,
    operation_gate: Mutex<()>,
}

impl MatrixCredentialRuntime {
    pub(crate) fn system(config: &DesktopBridgeConfig) -> Self {
        Self::new(Arc::new(KeyringMatrixCredentialVault {
            service: storage_service(
                config.secure_storage_service().as_str(),
                config.matrix_base_url().as_str(),
            ),
        }))
    }

    fn new(vault: Arc<dyn MatrixCredentialVault>) -> Self {
        Self {
            vault,
            operation_gate: Mutex::new(()),
        }
    }

    pub(crate) fn load(&self) -> Result<Option<StoredMatrixSession>, MatrixSessionFailure> {
        let _guard = self.operation_gate.lock().map_err(|_| unavailable())?;
        let Some(serialized) = self.vault.load()? else {
            return Ok(None);
        };
        let session: StoredMatrixSession =
            serde_json::from_str(&serialized).map_err(|_| corrupt())?;
        session.validate().map_err(|_| corrupt())?;
        Ok(Some(session))
    }

    pub(crate) fn save(&self, session: &StoredMatrixSession) -> Result<(), MatrixSessionFailure> {
        session.validate()?;
        let serialized = serde_json::to_string(session).map_err(|_| {
            MatrixSessionFailure::new("desktop.matrix_session.serialize_failed", false)
        })?;
        let _guard = self.operation_gate.lock().map_err(|_| unavailable())?;
        self.vault.save(&serialized)
    }

    pub(crate) fn clear(&self) -> Result<(), MatrixSessionFailure> {
        let _guard = self.operation_gate.lock().map_err(|_| unavailable())?;
        self.vault.clear()
    }
}

fn valid_value(value: &str, max_length: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_length
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// 命名空间随环境和 Homeserver 隔离，不随安装目录或应用版本变化。
fn storage_service(environment: &str, homeserver: &str) -> String {
    let digest = Sha256::digest(format!("{environment}\0{homeserver}").as_bytes());
    format!(
        "dev.agent-room.desktop-matrix.{}",
        URL_SAFE_NO_PAD.encode(digest)
    )
}

const fn unavailable() -> MatrixSessionFailure {
    MatrixSessionFailure::new("desktop.matrix_session.vault_unavailable", true)
}

const fn corrupt() -> MatrixSessionFailure {
    MatrixSessionFailure::new("desktop.matrix_session.vault_corrupt", false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MemoryVault(Mutex<Option<String>>);

    impl MatrixCredentialVault for MemoryVault {
        fn load(&self) -> Result<Option<String>, MatrixSessionFailure> {
            Ok(self.0.lock().expect("测试存储未中毒").clone())
        }

        fn save(&self, serialized: &str) -> Result<(), MatrixSessionFailure> {
            *self.0.lock().expect("测试存储未中毒") = Some(serialized.to_owned());
            Ok(())
        }

        fn clear(&self) -> Result<(), MatrixSessionFailure> {
            *self.0.lock().expect("测试存储未中毒") = None;
            Ok(())
        }
    }

    fn session() -> StoredMatrixSession {
        StoredMatrixSession {
            access_token: "test-access".to_owned(),
            refresh_token: Some("test-refresh".to_owned()),
            device_id: "TEST_DEVICE".to_owned(),
            user_id: "@tester:matrix.test".to_owned(),
            version: 1,
        }
    }

    #[test]
    fn 重建运行时保留用户和设备但退出清理后不再恢复() {
        let vault = Arc::new(MemoryVault::default());
        let first = MatrixCredentialRuntime::new(vault.clone());
        assert!(first.load().expect("空存储可读").is_none());
        first.save(&session()).expect("会话可写");
        drop(first);
        let second = MatrixCredentialRuntime::new(vault);
        assert!(second.load().expect("可恢复会话") == Some(session()));
        second.clear().expect("可清理会话");
        second.clear().expect("重复清理幂等");
        assert!(second.load().expect("可读取清理结果").is_none());
    }

    #[test]
    fn 损坏存储返回稳定错误而不是伪造未登录() {
        let vault = Arc::new(MemoryVault::default());
        let runtime = MatrixCredentialRuntime::new(vault.clone());
        for serialized in ["not-json", "{}", r#"{"version":2}"#] {
            vault.save(serialized).expect("可准备损坏记录");
            assert!(matches!(runtime.load(), Err(error) if error == corrupt()));
        }
    }

    #[test]
    fn 非法会话不能覆盖原有凭据() {
        let runtime = MatrixCredentialRuntime::new(Arc::new(MemoryVault::default()));
        runtime.save(&session()).expect("可写入初始会话");
        let mut invalid = session();
        invalid.access_token = "has a space".to_owned();
        assert!(runtime.save(&invalid).is_err());
        assert!(runtime.load().expect("旧会话仍可读") == Some(session()));
        let mut value = serde_json::to_value(session()).expect("可序列化会话");
        value["unexpected"] = true.into();
        assert!(serde_json::from_value::<StoredMatrixSession>(value).is_err());
    }

    #[test]
    fn 环境和服务器分别隔离存储命名空间() {
        assert_ne!(
            storage_service("prod", "https://one/"),
            storage_service("dev", "https://one/")
        );
        assert_ne!(
            storage_service("prod", "https://one/"),
            storage_service("prod", "https://two/")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_系统凭据库跨进程恢复验收() {
        const TEST_SERVICE: &str = "AGENT_ROOM_TEST_MATRIX_VAULT_SERVICE";
        if let Ok(service) = std::env::var(TEST_SERVICE) {
            assert!(service.starts_with("dev.agent-room.test.matrix."));
            let runtime =
                MatrixCredentialRuntime::new(Arc::new(KeyringMatrixCredentialVault { service }));
            assert!(runtime.load().expect("新进程可恢复会话") == Some(session()));
            return;
        }
        // 只读写随机测试命名空间，不接触当前用户的产品会话。
        let service = format!("dev.agent-room.test.matrix.{}", uuid::Uuid::now_v7());
        let runtime = MatrixCredentialRuntime::new(Arc::new(KeyringMatrixCredentialVault {
            service: service.clone(),
        }));
        runtime.save(&session()).expect("Windows 凭据库可写入");
        drop(runtime);
        let child = std::process::Command::new(std::env::current_exe().expect("可定位测试程序"))
            .args([
                "--exact",
                "matrix_credentials::tests::windows_系统凭据库跨进程恢复验收",
            ])
            .env(TEST_SERVICE, &service)
            .output();
        let restored =
            MatrixCredentialRuntime::new(Arc::new(KeyringMatrixCredentialVault { service }));
        let result = restored.load();
        restored.clear().expect("清理本次测试凭据");
        assert!(child.expect("测试子进程可运行").status.success());
        assert!(result.expect("Windows 凭据库可恢复") == Some(session()));
    }
}
