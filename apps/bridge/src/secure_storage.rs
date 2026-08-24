use std::sync::Arc;

use agent_room_application::ports::{
    DeviceSignature, MatrixDeviceId, MatrixSession, MatrixSessionMetadata, MatrixUserId,
    SecretValue,
};
use agent_room_bridge_core::ports::{
    BridgeCredentialFailure, BridgeCredentialFailureKind, BridgeCredentialResult,
    BridgeCredentialState, DeviceCredentialVault, DeviceSigningIdentity,
    DeviceSigningIdentityStore, StoredBridgeDeviceCredentials,
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    agent_runtime::{
        AgentRuntimeCredentialVault, AgentRuntimeRegistrationIntent, RegisteredAgentRuntime,
        StoredAgentRuntimeCredentials,
    },
    ipc::IpcInstallationId,
};
use agent_room_bridge_ipc::IpcSharedSecret;
use agent_room_bridge_local_adapter::{
    IPC_INSTALLATION_ID_ACCOUNT as IPC_INSTALLATION_ID,
    IPC_SHARED_SECRET_ACCOUNT as IPC_SHARED_SECRET,
};
use agent_room_bridge_storage_adapter::HandoffStorageKey;
use agent_room_domain::{
    agents::AgentInstancePublicSigningKey,
    devices::DevicePublicSigningKey,
    ids::{
        AdapterBindingId, AgentId, AgentInstanceId, AgentInstanceRegistrationRequestId, DeviceId,
    },
    time::UtcMillis,
};
use agent_room_identity_adapter::{DeviceSigningKeyError, Ed25519DeviceSigningKey};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const DEVICE_SIGNING_SEED: &str = "device-signing-seed";
const AGENT_INSTANCE_SIGNING_SEED: &str = "agent-instance-signing-seed-v1";
const DEVICE_CREDENTIALS: &str = "device-session-v1";
const AGENT_RUNTIME_CREDENTIALS: &str = "agent-runtime-session-v1";
const MATRIX_STORE_PASSPHRASE: &str = "matrix-store-passphrase-v1";
const HANDOFF_STORAGE_KEY: &str = "handoff-storage-key-v1";
const CREDENTIAL_FORMAT_VERSION: u8 = 1;
const RUNTIME_SECRET_BYTES: usize = 32;

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

pub(crate) struct BridgeRuntimeSecrets {
    installation_id: IpcInstallationId,
    ipc_shared_secret: IpcSharedSecret,
    matrix_store_passphrase: SecretValue,
    handoff_storage_key: HandoffStorageKey,
}

impl BridgeRuntimeSecrets {
    pub(crate) const fn installation_id(&self) -> &IpcInstallationId {
        &self.installation_id
    }

    pub(crate) const fn ipc_shared_secret(&self) -> &IpcSharedSecret {
        &self.ipc_shared_secret
    }

    pub(crate) const fn matrix_store_passphrase(&self) -> &SecretValue {
        &self.matrix_store_passphrase
    }

    pub(crate) const fn handoff_storage_key(&self) -> &HandoffStorageKey {
        &self.handoff_storage_key
    }
}

/// 负责 Bridge 运行时安装身份与持久存储口令的 OS 安全存储适配器。
///
/// 它只保存最小秘密，不保存 Matrix 会话正文、宿主路径或可导出的明文配置。
pub(crate) struct OsBridgeRuntimeSecretVault {
    backend: Arc<dyn SecretStoreBackend>,
}

impl OsBridgeRuntimeSecretVault {
    pub(crate) fn system(service: impl Into<String>) -> Self {
        Self {
            backend: Arc::new(KeyringBackend::new(service)),
        }
    }

    #[cfg(test)]
    fn new(backend: Arc<dyn SecretStoreBackend>) -> Self {
        Self { backend }
    }

    /// 读取稳定安装身份；首次启动时生成并回读所有运行时秘密。
    ///
    /// # Errors
    ///
    /// 系统熵、安全存储或持久值校验失败时返回明确凭据错误。
    pub(crate) fn load_or_create(&self) -> BridgeCredentialResult<BridgeRuntimeSecrets> {
        let installation_id =
            self.load_or_create_value(IPC_INSTALLATION_ID, || Ok(Uuid::now_v7().to_string()))?;
        let shared_secret = self.load_or_create_value(IPC_SHARED_SECRET, random_secret)?;
        let matrix_store_passphrase =
            self.load_or_create_value(MATRIX_STORE_PASSPHRASE, random_secret)?;
        let handoff_storage_key = self.load_or_create_value(HANDOFF_STORAGE_KEY, random_secret)?;

        Ok(BridgeRuntimeSecrets {
            installation_id: IpcInstallationId::new(installation_id).map_err(|_| corrupt())?,
            ipc_shared_secret: IpcSharedSecret::new(decode_runtime_secret(&shared_secret)?),
            matrix_store_passphrase: SecretValue::new(matrix_store_passphrase)
                .map_err(|_| corrupt())?,
            handoff_storage_key: HandoffStorageKey::from_bytes(decode_runtime_secret(
                &handoff_storage_key,
            )?),
        })
    }

    fn load_or_create_value(
        &self,
        account: &str,
        create: impl FnOnce() -> BridgeCredentialResult<String>,
    ) -> BridgeCredentialResult<String> {
        if let Some(value) = self.backend.read(account)? {
            return Ok(value);
        }

        self.backend.write(account, &create()?)?;
        self.backend.read(account)?.ok_or_else(unavailable)
    }
}

fn random_secret() -> BridgeCredentialResult<String> {
    let mut secret = [0_u8; RUNTIME_SECRET_BYTES];
    getrandom::fill(&mut secret).map_err(|_| unavailable())?;
    Ok(URL_SAFE_NO_PAD.encode(secret))
}

fn decode_runtime_secret(encoded: &str) -> BridgeCredentialResult<[u8; RUNTIME_SECRET_BYTES]> {
    URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| corrupt())?
        .try_into()
        .map_err(|_| corrupt())
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
}

impl DeviceSigningIdentityStore for OsDeviceSigningIdentityStore {
    fn load_or_create(&self) -> BridgeCredentialResult<Arc<dyn DeviceSigningIdentity>> {
        load_or_create_signing_identity(self.backend.as_ref(), DEVICE_SIGNING_SEED)
    }
}

pub(crate) struct OsAgentInstanceSigningIdentityStore {
    backend: Arc<dyn SecretStoreBackend>,
}

impl OsAgentInstanceSigningIdentityStore {
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

impl DeviceSigningIdentityStore for OsAgentInstanceSigningIdentityStore {
    fn load_or_create(&self) -> BridgeCredentialResult<Arc<dyn DeviceSigningIdentity>> {
        load_or_create_signing_identity(self.backend.as_ref(), AGENT_INSTANCE_SIGNING_SEED)
    }
}

fn load_or_create_signing_identity(
    backend: &dyn SecretStoreBackend,
    account: &str,
) -> BridgeCredentialResult<Arc<dyn DeviceSigningIdentity>> {
    if let Some(encoded_seed) = backend.read(account)? {
        return decode_signing_identity(encoded_seed);
    }

    let key = Ed25519DeviceSigningKey::generate().map_err(map_signing_key_failure)?;
    let encoded_seed = key.encoded_seed().map_err(map_signing_key_failure)?;
    backend.write(account, encoded_seed.expose())?;
    let persisted_seed = backend.read(account)?.ok_or_else(unavailable)?;
    decode_signing_identity(persisted_seed)
}

fn decode_signing_identity(
    encoded_seed: String,
) -> BridgeCredentialResult<Arc<dyn DeviceSigningIdentity>> {
    let seed = SecretValue::new(encoded_seed).map_err(|_| corrupt())?;
    let key = Ed25519DeviceSigningKey::from_encoded_seed(&seed).map_err(map_signing_key_failure)?;
    Ok(Arc::new(Ed25519SigningIdentity(key)))
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

pub(crate) struct OsAgentRuntimeCredentialVault {
    backend: Arc<dyn SecretStoreBackend>,
}

impl OsAgentRuntimeCredentialVault {
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

impl AgentRuntimeCredentialVault for OsAgentRuntimeCredentialVault {
    fn load(&self) -> BridgeCredentialResult<Option<StoredAgentRuntimeCredentials>> {
        self.backend
            .read(AGENT_RUNTIME_CREDENTIALS)?
            .map(|serialized| decode_agent_runtime_credentials(&serialized))
            .transpose()
    }

    fn replace(&self, credentials: &StoredAgentRuntimeCredentials) -> BridgeCredentialResult<()> {
        self.backend.write(
            AGENT_RUNTIME_CREDENTIALS,
            &encode_agent_runtime_credentials(credentials)?,
        )
    }

    fn clear(&self) -> BridgeCredentialResult<()> {
        self.backend.delete(AGENT_RUNTIME_CREDENTIALS)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedDeviceCredentials {
    version: u8,
    #[serde(default = "ready_credential_state")]
    state: String,
    device_id: String,
    access_token: String,
    access_token_expires_at_unix_ms: i64,
    refresh_token: String,
    refresh_token_expires_at_unix_ms: i64,
}

fn ready_credential_state() -> String {
    "ready".to_owned()
}

fn encode_credentials(
    credentials: &StoredBridgeDeviceCredentials,
) -> BridgeCredentialResult<String> {
    serde_json::to_string(&PersistedDeviceCredentials {
        version: CREDENTIAL_FORMAT_VERSION,
        state: match credentials.state {
            BridgeCredentialState::Ready => "ready",
            BridgeCredentialState::RefreshPending => "refresh_pending",
        }
        .to_owned(),
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
        state: match persisted.state.as_str() {
            "ready" => BridgeCredentialState::Ready,
            "refresh_pending" => BridgeCredentialState::RefreshPending,
            _ => return Err(corrupt()),
        },
        device_id,
        access_token: SecretValue::new(persisted.access_token).map_err(|_| corrupt())?,
        access_token_expires_at: UtcMillis::new(persisted.access_token_expires_at_unix_ms)
            .map_err(|_| corrupt())?,
        refresh_token: SecretValue::new(persisted.refresh_token).map_err(|_| corrupt())?,
        refresh_token_expires_at: UtcMillis::new(persisted.refresh_token_expires_at_unix_ms)
            .map_err(|_| corrupt())?,
    })
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum PersistedAgentRuntimeCredentials {
    RegistrationPending {
        version: u8,
        intent: PersistedAgentRuntimeIntent,
    },
    Ready {
        version: u8,
        intent: PersistedAgentRuntimeIntent,
        runtime: PersistedRegisteredAgentRuntime,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PersistedAgentRuntimeIntent {
    request_id: String,
    agent_id: String,
    adapter_type: String,
    capability_version: String,
    public_signing_key: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PersistedRegisteredAgentRuntime {
    agent_id: String,
    display_name: String,
    agent_instance_id: String,
    adapter_binding_id: String,
    matrix_user_id: String,
    matrix_device_id: String,
    access_token: String,
    refresh_token: Option<String>,
}

fn encode_agent_runtime_credentials(
    credentials: &StoredAgentRuntimeCredentials,
) -> BridgeCredentialResult<String> {
    let persisted = match credentials {
        StoredAgentRuntimeCredentials::RegistrationPending(intent) => {
            PersistedAgentRuntimeCredentials::RegistrationPending {
                version: CREDENTIAL_FORMAT_VERSION,
                intent: persisted_intent(intent),
            }
        }
        StoredAgentRuntimeCredentials::Ready { intent, runtime } => {
            PersistedAgentRuntimeCredentials::Ready {
                version: CREDENTIAL_FORMAT_VERSION,
                intent: persisted_intent(intent),
                runtime: PersistedRegisteredAgentRuntime {
                    agent_id: runtime.identity().agent_id().to_string(),
                    display_name: runtime.identity().display_name().to_owned(),
                    agent_instance_id: runtime.identity().agent_instance_id().to_string(),
                    adapter_binding_id: runtime.adapter_binding_id().to_string(),
                    matrix_user_id: runtime
                        .matrix_session()
                        .metadata()
                        .user_id()
                        .as_str()
                        .to_owned(),
                    matrix_device_id: runtime
                        .matrix_session()
                        .metadata()
                        .device_id()
                        .as_str()
                        .to_owned(),
                    access_token: runtime.matrix_session().access_token().expose().to_owned(),
                    refresh_token: runtime
                        .matrix_session()
                        .refresh_token()
                        .map(|token| token.expose().to_owned()),
                },
            }
        }
    };
    serde_json::to_string(&persisted).map_err(|_| corrupt())
}

fn persisted_intent(intent: &AgentRuntimeRegistrationIntent) -> PersistedAgentRuntimeIntent {
    PersistedAgentRuntimeIntent {
        request_id: intent.request_id().to_string(),
        agent_id: intent.agent_id().to_string(),
        adapter_type: intent.adapter_type().to_owned(),
        capability_version: intent.capability_version().to_owned(),
        public_signing_key: URL_SAFE_NO_PAD.encode(intent.public_signing_key().as_bytes()),
    }
}

fn decode_agent_runtime_credentials(
    serialized: &str,
) -> BridgeCredentialResult<StoredAgentRuntimeCredentials> {
    let persisted = serde_json::from_str::<PersistedAgentRuntimeCredentials>(serialized)
        .map_err(|_| corrupt())?;
    match persisted {
        PersistedAgentRuntimeCredentials::RegistrationPending { version, intent } => {
            ensure_credential_version(version)?;
            Ok(StoredAgentRuntimeCredentials::RegistrationPending(
                decode_agent_runtime_intent(intent)?,
            ))
        }
        PersistedAgentRuntimeCredentials::Ready {
            version,
            intent,
            runtime,
        } => {
            ensure_credential_version(version)?;
            let intent = decode_agent_runtime_intent(intent)?;
            let runtime = decode_registered_agent_runtime(runtime)?;
            if intent.agent_id() != runtime.identity().agent_id() {
                return Err(corrupt());
            }
            Ok(StoredAgentRuntimeCredentials::Ready {
                intent,
                runtime: Box::new(runtime),
            })
        }
    }
}

fn ensure_credential_version(version: u8) -> BridgeCredentialResult<()> {
    if version == CREDENTIAL_FORMAT_VERSION {
        Ok(())
    } else {
        Err(corrupt())
    }
}

fn decode_agent_runtime_intent(
    persisted: PersistedAgentRuntimeIntent,
) -> BridgeCredentialResult<AgentRuntimeRegistrationIntent> {
    let public_signing_key = URL_SAFE_NO_PAD
        .decode(persisted.public_signing_key)
        .map_err(|_| corrupt())
        .and_then(|bytes| AgentInstancePublicSigningKey::new(bytes).map_err(|_| corrupt()))?;
    AgentRuntimeRegistrationIntent::new(
        parse_v7_id(&persisted.request_id).map(AgentInstanceRegistrationRequestId::from_uuid)?,
        parse_v7_id(&persisted.agent_id).map(AgentId::from_uuid)?,
        persisted.adapter_type,
        persisted.capability_version,
        public_signing_key,
    )
    .map_err(|_| corrupt())
}

fn decode_registered_agent_runtime(
    persisted: PersistedRegisteredAgentRuntime,
) -> BridgeCredentialResult<RegisteredAgentRuntime> {
    let agent_id = parse_v7_id(&persisted.agent_id).map(AgentId::from_uuid)?;
    let instance_id = parse_v7_id(&persisted.agent_instance_id).map(AgentInstanceId::from_uuid)?;
    let binding_id = parse_v7_id(&persisted.adapter_binding_id).map(AdapterBindingId::from_uuid)?;
    let matrix_user_id = MatrixUserId::new(persisted.matrix_user_id).map_err(|_| corrupt())?;
    let identity = BridgeAgentIdentity::new(
        agent_id,
        persisted.display_name,
        matrix_user_id.as_str(),
        instance_id,
    )
    .map_err(|_| corrupt())?;
    let matrix_device_id =
        MatrixDeviceId::new(persisted.matrix_device_id).map_err(|_| corrupt())?;
    let access_token = SecretValue::new(persisted.access_token).map_err(|_| corrupt())?;
    let refresh_token = persisted
        .refresh_token
        .map(SecretValue::new)
        .transpose()
        .map_err(|_| corrupt())?;
    RegisteredAgentRuntime::new(
        identity,
        binding_id,
        MatrixSession::new(
            MatrixSessionMetadata::new(matrix_user_id, matrix_device_id),
            access_token,
            refresh_token,
        ),
    )
    .map_err(|_| corrupt())
}

fn parse_v7_id(value: &str) -> BridgeCredentialResult<Uuid> {
    let id = Uuid::parse_str(value).map_err(|_| corrupt())?;
    if id.get_version() == Some(uuid::Version::SortRand) {
        Ok(id)
    } else {
        Err(corrupt())
    }
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

    use agent_room_application::ports::{
        MatrixDeviceId, MatrixSession, MatrixSessionMetadata, MatrixUserId, SecretValue,
    };
    use agent_room_bridge_core::{
        agent_identity::BridgeAgentIdentity,
        agent_runtime::{
            AgentRuntimeCredentialVault, AgentRuntimeRegistrationIntent, RegisteredAgentRuntime,
            StoredAgentRuntimeCredentials,
        },
        ports::{
            BridgeCredentialFailureKind, BridgeCredentialState, DeviceCredentialVault,
            DeviceSigningIdentityStore, StoredBridgeDeviceCredentials,
        },
    };
    use agent_room_domain::{
        agents::AgentInstancePublicSigningKey,
        ids::{
            AdapterBindingId, AgentId, AgentInstanceId, AgentInstanceRegistrationRequestId,
            DeviceId,
        },
        time::UtcMillis,
    };
    use uuid::Uuid;

    use super::{
        AGENT_INSTANCE_SIGNING_SEED, AGENT_RUNTIME_CREDENTIALS, DEVICE_CREDENTIALS,
        DEVICE_SIGNING_SEED, HANDOFF_STORAGE_KEY, IPC_INSTALLATION_ID, IPC_SHARED_SECRET,
        KeyringBackend, MATRIX_STORE_PASSPHRASE, OsAgentInstanceSigningIdentityStore,
        OsAgentRuntimeCredentialVault, OsBridgeRuntimeSecretVault, OsDeviceCredentialVault,
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
    fn agent_实例私钥与用户设备私钥使用独立安全存储槽位() {
        let backend = Arc::new(内存安全存储::default());
        let device = OsDeviceSigningIdentityStore::new(backend.clone());
        let agent = OsAgentInstanceSigningIdentityStore::new(backend.clone());

        let device_key = device.load_or_create().expect("设备私钥可生成");
        let agent_key = agent.load_or_create().expect("Agent 私钥可生成");

        assert_ne!(
            device_key.public_key().expect("设备公钥可导出"),
            agent_key.public_key().expect("Agent 公钥可导出")
        );
        let stored = backend.0.lock().expect("存储锁未中毒");
        assert!(stored.contains_key(DEVICE_SIGNING_SEED));
        assert!(stored.contains_key(AGENT_INSTANCE_SIGNING_SEED));
    }

    #[test]
    fn 运行时秘密首次生成后保持稳定且彼此隔离() {
        let backend = Arc::new(内存安全存储::default());
        let vault = OsBridgeRuntimeSecretVault::new(backend.clone());

        let first = vault.load_or_create().expect("首次运行时秘密可生成");
        let second = vault.load_or_create().expect("运行时秘密可恢复");

        assert_eq!(first.installation_id(), second.installation_id());
        let stored = backend.0.lock().expect("存储锁未中毒");
        assert!(stored.contains_key(IPC_INSTALLATION_ID));
        assert!(stored.contains_key(IPC_SHARED_SECRET));
        assert!(stored.contains_key(MATRIX_STORE_PASSPHRASE));
        assert!(stored.contains_key(HANDOFF_STORAGE_KEY));
        assert_ne!(stored[IPC_SHARED_SECRET], stored[MATRIX_STORE_PASSPHRASE]);
        assert_ne!(stored[HANDOFF_STORAGE_KEY], stored[MATRIX_STORE_PASSPHRASE]);
        assert_ne!(stored[HANDOFF_STORAGE_KEY], stored[IPC_SHARED_SECRET]);
    }

    #[test]
    fn 畸形运行时秘密不会被静默替换() {
        let backend = Arc::new(内存安全存储::default());
        backend
            .write(IPC_SHARED_SECRET, "不是合法密钥")
            .expect("测试值可写入");
        let vault = OsBridgeRuntimeSecretVault::new(backend);

        let Err(failure) = vault.load_or_create() else {
            panic!("畸形秘密必须阻止启动");
        };

        assert_eq!(failure.kind(), BridgeCredentialFailureKind::Corrupt);
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
    fn 旧版凭据缺少刷新状态时按可用状态迁移() {
        let backend = Arc::new(内存安全存储::default());
        backend
            .write(
                DEVICE_CREDENTIALS,
                r#"{"version":1,"device_id":"00000000-0000-0000-0000-000000000001","access_token":"access-token","access_token_expires_at_unix_ms":2000,"refresh_token":"refresh-token","refresh_token_expires_at_unix_ms":3000}"#,
            )
            .expect("旧版测试值可写入");
        let vault = OsDeviceCredentialVault::new(backend);

        let credentials = vault.load().expect("旧版凭据可迁移").expect("旧版凭据存在");

        assert_eq!(credentials.state, BridgeCredentialState::Ready);
    }

    #[test]
    fn agent_登记意图与就绪会话作为版本化秘密往返() {
        let backend = Arc::new(内存安全存储::default());
        let vault = OsAgentRuntimeCredentialVault::new(backend.clone());
        let intent = agent_runtime_intent();
        let pending = StoredAgentRuntimeCredentials::RegistrationPending(intent.clone());

        vault.replace(&pending).expect("登记意图可写入");
        assert_eq!(vault.load().expect("登记意图可读取"), Some(pending));

        let ready = StoredAgentRuntimeCredentials::Ready {
            intent,
            runtime: Box::new(registered_agent_runtime()),
        };
        vault.replace(&ready).expect("就绪会话可原子替换");
        assert_eq!(vault.load().expect("就绪会话可读取"), Some(ready));
        vault.clear().expect("Agent 运行凭据可清理");
        assert_eq!(vault.load().expect("空存储可读取"), None);
        assert!(
            !backend
                .0
                .lock()
                .expect("存储锁未中毒")
                .contains_key(AGENT_RUNTIME_CREDENTIALS)
        );
    }

    #[test]
    fn agent_运行凭据身份错配时拒绝恢复() {
        let backend = Arc::new(内存安全存储::default());
        let mut serialized =
            super::encode_agent_runtime_credentials(&StoredAgentRuntimeCredentials::Ready {
                intent: agent_runtime_intent(),
                runtime: Box::new(registered_agent_runtime()),
            })
            .expect("测试凭据可编码");
        serialized = serialized.replacen(
            "0198b601-77a1-7bb8-83eb-a8fe68c97e44",
            "0198b601-77a1-7bb8-83eb-a8fe68c97e45",
            1,
        );
        backend
            .write(AGENT_RUNTIME_CREDENTIALS, &serialized)
            .expect("篡改值可写入");
        let vault = OsAgentRuntimeCredentialVault::new(backend);

        let failure = vault.load().expect_err("身份错配必须失败");

        assert_eq!(failure.kind(), BridgeCredentialFailureKind::Corrupt);
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
            state: BridgeCredentialState::Ready,
            device_id: DeviceId::from_uuid(Uuid::from_u128(1)),
            access_token: SecretValue::new("access-token").expect("测试 Token 有效"),
            access_token_expires_at: UtcMillis::new(2_000).expect("测试时间有效"),
            refresh_token: SecretValue::new("refresh-token").expect("测试 Token 有效"),
            refresh_token_expires_at: UtcMillis::new(3_000).expect("测试时间有效"),
        }
    }

    fn agent_runtime_intent() -> AgentRuntimeRegistrationIntent {
        AgentRuntimeRegistrationIntent::new(
            AgentInstanceRegistrationRequestId::from_uuid(
                Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e49").expect("测试 UUID 有效"),
            ),
            agent_id(),
            "codex-desktop",
            "2026-08-24",
            AgentInstancePublicSigningKey::new(vec![7; 32]).expect("测试公钥有效"),
        )
        .expect("测试登记意图有效")
    }

    fn registered_agent_runtime() -> RegisteredAgentRuntime {
        let matrix_user_id = MatrixUserId::new("@agent:example.org").expect("Matrix 用户标识有效");
        let identity = BridgeAgentIdentity::new(
            agent_id(),
            "Codex Builder",
            matrix_user_id.as_str(),
            AgentInstanceId::from_uuid(
                Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e47").expect("测试 UUID 有效"),
            ),
        )
        .expect("Agent 身份有效");
        RegisteredAgentRuntime::new(
            identity,
            AdapterBindingId::from_uuid(
                Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e48").expect("测试 UUID 有效"),
            ),
            MatrixSession::new(
                MatrixSessionMetadata::new(
                    matrix_user_id,
                    MatrixDeviceId::new("AR_TEST").expect("Matrix 设备标识有效"),
                ),
                SecretValue::new("agent-access-token").expect("测试 Token 有效"),
                Some(SecretValue::new("agent-refresh-token").expect("测试 Token 有效")),
            ),
        )
        .expect("Agent 运行时有效")
    }

    fn agent_id() -> AgentId {
        AgentId::from_uuid(
            Uuid::parse_str("0198b601-77a1-7bb8-83eb-a8fe68c97e44").expect("测试 UUID 有效"),
        )
    }
}
