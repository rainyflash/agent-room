use std::sync::Arc;

use agent_room_application::ports::{DeviceSignature, SecretValue};
use agent_room_bridge_core::ports::{
    BridgeCredentialFailure, BridgeCredentialFailureKind, BridgeCredentialResult,
    DeviceCredentialVault, DeviceSigningIdentity, DeviceSigningIdentityStore,
    StoredBridgeDeviceCredentials,
};
use agent_room_domain::{devices::DevicePublicSigningKey, ids::DeviceId, time::UtcMillis};
use agent_room_identity_adapter::{DeviceSigningKeyError, Ed25519DeviceSigningKey};
use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const DEVICE_SIGNING_SEED: &str = "device-signing-seed";
const DEVICE_CREDENTIALS: &str = "device-session-v1";
const CREDENTIAL_FORMAT_VERSION: u8 = 1;

trait SecretStoreBackend: Send + Sync {
    fn read(&self, account: &str) -> BridgeCredentialResult<Option<String>>;
    fn write(&self, account: &str, value: &str) -> BridgeCredentialResult<()>;
    fn delete(&self, account: &str) -> BridgeCredentialResult<()>;
}

struct KeyringBackend {
    service: String,
}

impl KeyringBackend {
    fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, account: &str) -> BridgeCredentialResult<Entry> {
        Entry::new(&self.service, account).map_err(|_| unavailable())
    }
}

impl SecretStoreBackend for KeyringBackend {
    fn read(&self, account: &str) -> BridgeCredentialResult<Option<String>> {
        match self.entry(account)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(_) => Err(unavailable()),
        }
    }

    fn write(&self, account: &str, value: &str) -> BridgeCredentialResult<()> {
        self.entry(account)?
            .set_password(value)
            .map_err(|_| unavailable())
    }

    fn delete(&self, account: &str) -> BridgeCredentialResult<()> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(_) => Err(unavailable()),
        }
    }
}

pub(crate) struct OsDeviceSigningIdentityStore {
    backend: Arc<dyn SecretStoreBackend>,
}

impl OsDeviceSigningIdentityStore {
    pub(crate) fn system(service: impl Into<String>) -> Self {
        Self {
            backend: Arc::new(KeyringBackend::new(service)),
        }
    }

    #[cfg(test)]
    fn new(backend: Arc<dyn SecretStoreBackend>) -> Self {
        Self { backend }
    }

    fn decode(encoded_seed: String) -> BridgeCredentialResult<Arc<dyn DeviceSigningIdentity>> {
        let seed = SecretValue::new(encoded_seed).map_err(|_| corrupt())?;
        let key =
            Ed25519DeviceSigningKey::from_encoded_seed(&seed).map_err(map_signing_key_failure)?;
        Ok(Arc::new(Ed25519SigningIdentity(key)))
    }
}

impl DeviceSigningIdentityStore for OsDeviceSigningIdentityStore {
    fn load_or_create(&self) -> BridgeCredentialResult<Arc<dyn DeviceSigningIdentity>> {
        if let Some(encoded_seed) = self.backend.read(DEVICE_SIGNING_SEED)? {
            return Self::decode(encoded_seed);
        }

        let key = Ed25519DeviceSigningKey::generate().map_err(map_signing_key_failure)?;
        let encoded_seed = key.encoded_seed().map_err(map_signing_key_failure)?;
        self.backend
            .write(DEVICE_SIGNING_SEED, encoded_seed.expose())?;
        let persisted_seed = self
            .backend
            .read(DEVICE_SIGNING_SEED)?
            .ok_or_else(unavailable)?;
        Self::decode(persisted_seed)
    }
}

struct Ed25519SigningIdentity(Ed25519DeviceSigningKey);

impl DeviceSigningIdentity for Ed25519SigningIdentity {
    fn public_key(&self) -> BridgeCredentialResult<DevicePublicSigningKey> {
        self.0.public_key().map_err(map_signing_key_failure)
    }

    fn sign(&self, message: &[u8]) -> BridgeCredentialResult<DeviceSignature> {
        self.0.sign(message).map_err(map_signing_key_failure)
    }
}

pub(crate) struct OsDeviceCredentialVault {
    backend: Arc<dyn SecretStoreBackend>,
}

impl OsDeviceCredentialVault {
    pub(crate) fn system(service: impl Into<String>) -> Self {
        Self {
            backend: Arc::new(KeyringBackend::new(service)),
        }
    }

    #[cfg(test)]
    fn new(backend: Arc<dyn SecretStoreBackend>) -> Self {
        Self { backend }
    }
}

impl DeviceCredentialVault for OsDeviceCredentialVault {
    fn load(&self) -> BridgeCredentialResult<Option<StoredBridgeDeviceCredentials>> {
        self.backend
            .read(DEVICE_CREDENTIALS)?
            .map(|serialized| decode_credentials(&serialized))
            .transpose()
    }

    fn replace(&self, credentials: &StoredBridgeDeviceCredentials) -> BridgeCredentialResult<()> {
        let serialized = encode_credentials(credentials)?;
        self.backend.write(DEVICE_CREDENTIALS, &serialized)
    }

    fn clear(&self) -> BridgeCredentialResult<()> {
        self.backend.delete(DEVICE_CREDENTIALS)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedDeviceCredentials {
    version: u8,
    device_id: String,
    access_token: String,
    access_token_expires_at_unix_ms: i64,
    refresh_token: String,
    refresh_token_expires_at_unix_ms: i64,
}

fn encode_credentials(
    credentials: &StoredBridgeDeviceCredentials,
) -> BridgeCredentialResult<String> {
    serde_json::to_string(&PersistedDeviceCredentials {
        version: CREDENTIAL_FORMAT_VERSION,
        device_id: credentials.device_id.to_string(),
        access_token: credentials.access_token.expose().to_owned(),
        access_token_expires_at_unix_ms: credentials.access_token_expires_at.value(),
        refresh_token: credentials.refresh_token.expose().to_owned(),
        refresh_token_expires_at_unix_ms: credentials.refresh_token_expires_at.value(),
    })
    .map_err(|_| corrupt())
}

fn decode_credentials(serialized: &str) -> BridgeCredentialResult<StoredBridgeDeviceCredentials> {
    let persisted =
        serde_json::from_str::<PersistedDeviceCredentials>(serialized).map_err(|_| corrupt())?;
    if persisted.version != CREDENTIAL_FORMAT_VERSION {
        return Err(corrupt());
    }
    let device_id = Uuid::parse_str(&persisted.device_id)
        .map(DeviceId::from_uuid)
        .map_err(|_| corrupt())?;
    Ok(StoredBridgeDeviceCredentials {
        device_id,
        access_token: SecretValue::new(persisted.access_token).map_err(|_| corrupt())?,
        access_token_expires_at: UtcMillis::new(persisted.access_token_expires_at_unix_ms)
            .map_err(|_| corrupt())?,
        refresh_token: SecretValue::new(persisted.refresh_token).map_err(|_| corrupt())?,
        refresh_token_expires_at: UtcMillis::new(persisted.refresh_token_expires_at_unix_ms)
            .map_err(|_| corrupt())?,
    })
}

const fn map_signing_key_failure(error: DeviceSigningKeyError) -> BridgeCredentialFailure {
    match error {
        DeviceSigningKeyError::EntropyUnavailable => unavailable(),
        DeviceSigningKeyError::InvalidSeed | DeviceSigningKeyError::InvalidDerivedValue => {
            corrupt()
        }
    }
}

const fn unavailable() -> BridgeCredentialFailure {
    BridgeCredentialFailure::new(BridgeCredentialFailureKind::Unavailable)
}

const fn corrupt() -> BridgeCredentialFailure {
    BridgeCredentialFailure::new(BridgeCredentialFailureKind::Corrupt)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use agent_room_bridge_core::ports::{
        BridgeCredentialFailureKind, DeviceCredentialVault, DeviceSigningIdentityStore,
        StoredBridgeDeviceCredentials,
    };
    use agent_room_domain::{ids::DeviceId, time::UtcMillis};
    use uuid::Uuid;

    use super::{
        DEVICE_CREDENTIALS, DEVICE_SIGNING_SEED, KeyringBackend, OsDeviceCredentialVault,
        OsDeviceSigningIdentityStore, SecretStoreBackend, corrupt,
    };

    #[derive(Default)]
    struct 内存安全存储(Mutex<HashMap<String, String>>);

    impl SecretStoreBackend for 内存安全存储 {
        fn read(
            &self,
            account: &str,
        ) -> agent_room_bridge_core::ports::BridgeCredentialResult<Option<String>> {
            Ok(self.0.lock().expect("存储锁未中毒").get(account).cloned())
        }

        fn write(
            &self,
            account: &str,
            value: &str,
        ) -> agent_room_bridge_core::ports::BridgeCredentialResult<()> {
            self.0
                .lock()
                .expect("存储锁未中毒")
                .insert(account.to_owned(), value.to_owned());
            Ok(())
        }

        fn delete(
            &self,
            account: &str,
        ) -> agent_room_bridge_core::ports::BridgeCredentialResult<()> {
            self.0.lock().expect("存储锁未中毒").remove(account);
            Ok(())
        }
    }

    #[test]
    fn 设备私钥首次生成后只从安全存储恢复() {
        let backend = Arc::new(内存安全存储::default());
        let store = OsDeviceSigningIdentityStore::new(backend.clone());
        let first = store.load_or_create().expect("首次密钥可生成");
        let second = store.load_or_create().expect("密钥可恢复");

        assert_eq!(
            first.public_key().expect("公钥可导出"),
            second.public_key().expect("公钥可导出")
        );
        assert!(
            backend
                .0
                .lock()
                .expect("存储锁未中毒")
                .contains_key(DEVICE_SIGNING_SEED)
        );
    }

    #[test]
    fn 设备会话作为单个版本化秘密原子往返() {
        let backend = Arc::new(内存安全存储::default());
        let vault = OsDeviceCredentialVault::new(backend.clone());
        let credentials = credentials();

        vault.replace(&credentials).expect("凭据可写入");
        assert_eq!(vault.load().expect("凭据可读取"), Some(credentials));
        vault.clear().expect("凭据可删除");
        assert_eq!(vault.load().expect("空存储可读取"), None);
        assert!(
            !backend
                .0
                .lock()
                .expect("存储锁未中毒")
                .contains_key(DEVICE_CREDENTIALS)
        );
    }

    #[test]
    fn 安全存储中的畸形值必须显式标记损坏() {
        let backend = Arc::new(内存安全存储::default());
        backend
            .write(DEVICE_CREDENTIALS, "{\"version\":99}")
            .expect("测试值可写入");
        let vault = OsDeviceCredentialVault::new(backend);

        let failure = vault.load().expect_err("畸形值必须失败");

        assert_eq!(failure.kind(), BridgeCredentialFailureKind::Corrupt);
        assert_eq!(corrupt().kind(), BridgeCredentialFailureKind::Corrupt);
    }

    #[test]
    #[ignore = "显式运行以验证当前 OS 的真实安全存储"]
    fn 当前操作系统安全存储可写入读取和清理() {
        let service = format!("agent-room-test-{}", Uuid::now_v7());
        let backend = Arc::new(KeyringBackend::new(service));
        let signing_store = OsDeviceSigningIdentityStore::new(backend.clone());
        let vault = OsDeviceCredentialVault::new(backend.clone());

        signing_store
            .load_or_create()
            .expect("OS 安全存储可保存私钥");
        vault
            .replace(&credentials())
            .expect("OS 安全存储可保存会话");
        assert!(vault.load().expect("OS 安全存储可读取").is_some());

        vault.clear().expect("会话可清理");
        backend.delete(DEVICE_SIGNING_SEED).expect("私钥可清理");
    }

    fn credentials() -> StoredBridgeDeviceCredentials {
        StoredBridgeDeviceCredentials {
            device_id: DeviceId::from_uuid(Uuid::from_u128(1)),
            access_token: agent_room_application::ports::SecretValue::new("access-token")
                .expect("测试 Token 有效"),
            access_token_expires_at: UtcMillis::new(2_000).expect("测试时间有效"),
            refresh_token: agent_room_application::ports::SecretValue::new("refresh-token")
                .expect("测试 Token 有效"),
            refresh_token_expires_at: UtcMillis::new(3_000).expect("测试时间有效"),
        }
    }
}
