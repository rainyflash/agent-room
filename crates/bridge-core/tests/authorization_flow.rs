use std::sync::{Arc, Mutex};

use agent_room_application::{
    devices::{AuthenticatedDevice, DeviceCredentials, canonical_device_registration_message},
    ports::{
        DeviceSignature, OidcDeviceAuthorizationPrompt, OidcDeviceAuthorizationPromptSink,
        OidcDeviceGrantGateway, OidcDevicePromptFailure, OidcResult, PortFuture, PrincipalAccount,
        ProfileImportConsent, SecretFactory, SecretValue,
    },
};
use agent_room_bridge_core::{
    authorization::{
        AuthorizeBridgeDevice, BridgeAuthorizationDependencies, BridgeAuthorizationFailureKind,
        BridgeAuthorizationService,
    },
    ports::{
        BridgeCredentialFailure, BridgeCredentialFailureKind, BridgeCredentialResult,
        ControlPlaneDeviceGateway, ControlPlaneDeviceResult, DeviceCredentialVault,
        DeviceSigningIdentity, DeviceSigningIdentityStore, RegisterBridgeDevice,
        StoredBridgeDeviceCredentials,
    },
};
use agent_room_domain::{
    devices::{DevicePlatform, DevicePublicSigningKey},
    identity::Principal,
    ids::{DeviceId, PrincipalId},
    time::{DurationMillis, UtcMillis},
};
use agent_room_identity_adapter::SecureSecretFactory;
use uuid::Uuid;

const ASSERTION: &str = "header.payload.signature";

struct 测试Oidc;

impl OidcDeviceGrantGateway for 测试Oidc {
    fn authorize<'a>(
        &'a self,
        prompt_sink: &'a dyn OidcDeviceAuthorizationPromptSink,
    ) -> PortFuture<'a, OidcResult<SecretValue>> {
        Box::pin(async move {
            prompt_sink
                .present(&OidcDeviceAuthorizationPrompt {
                    user_code: SecretValue::new("ABCD-EFGH").expect("测试验证码有效"),
                    verification_uri: "https://identity.example/device".to_owned(),
                    verification_uri_complete: None,
                    expires_in: DurationMillis::new(60_000).expect("时长有效"),
                    polling_interval: DurationMillis::new(5_000).expect("时长有效"),
                })
                .map_err(|_| {
                    agent_room_application::ports::OidcFailure::new(
                        agent_room_application::ports::OidcFailureKind::ProviderRejected,
                    )
                })?;
            Ok(SecretValue::new(ASSERTION).expect("测试断言有效"))
        })
    }
}

struct 接受提示;

impl OidcDeviceAuthorizationPromptSink for 接受提示 {
    fn present(
        &self,
        _prompt: &OidcDeviceAuthorizationPrompt,
    ) -> Result<(), OidcDevicePromptFailure> {
        Ok(())
    }
}

struct 测试签名身份 {
    messages: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl DeviceSigningIdentity for 测试签名身份 {
    fn public_key(&self) -> BridgeCredentialResult<DevicePublicSigningKey> {
        Ok(DevicePublicSigningKey::new(vec![9; 32]).expect("测试公钥有效"))
    }

    fn sign(&self, message: &[u8]) -> BridgeCredentialResult<DeviceSignature> {
        self.messages
            .lock()
            .expect("签名消息锁未中毒")
            .push(message.to_vec());
        Ok(DeviceSignature::new(vec![7; 64]).expect("测试签名有效"))
    }
}

struct 测试签名存储 {
    identity: Arc<测试签名身份>,
}

impl DeviceSigningIdentityStore for 测试签名存储 {
    fn load_or_create(&self) -> BridgeCredentialResult<Arc<dyn DeviceSigningIdentity>> {
        Ok(self.identity.clone())
    }
}

#[derive(Default)]
struct 测试控制平面 {
    registrations: Mutex<Vec<RegisterBridgeDevice>>,
}

impl ControlPlaneDeviceGateway for 测试控制平面 {
    fn register(
        &self,
        request: RegisterBridgeDevice,
    ) -> PortFuture<'_, ControlPlaneDeviceResult<DeviceCredentials>> {
        self.registrations
            .lock()
            .expect("注册锁未中毒")
            .push(request);
        Box::pin(async { Ok(credentials()) })
    }
}

#[derive(Default)]
struct 内存凭据库 {
    value: Mutex<Option<StoredBridgeDeviceCredentials>>,
    reject_writes: bool,
}

impl DeviceCredentialVault for 内存凭据库 {
    fn load(&self) -> BridgeCredentialResult<Option<StoredBridgeDeviceCredentials>> {
        Ok(self.value.lock().expect("凭据锁未中毒").clone())
    }

    fn replace(&self, credentials: &StoredBridgeDeviceCredentials) -> BridgeCredentialResult<()> {
        if self.reject_writes {
            return Err(BridgeCredentialFailure::new(
                BridgeCredentialFailureKind::Unavailable,
            ));
        }
        self.value
            .lock()
            .expect("凭据锁未中毒")
            .replace(credentials.clone());
        Ok(())
    }

    fn clear(&self) -> BridgeCredentialResult<()> {
        self.value.lock().expect("凭据锁未中毒").take();
        Ok(())
    }
}

#[tokio::test]
async fn 设备授权把_oidc_断言_设备属性和公钥绑定在同一持有证明中() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let control_plane = Arc::new(测试控制平面::default());
    let vault = Arc::new(内存凭据库::default());
    let service = service(messages.clone(), control_plane.clone(), vault.clone());

    let authorized = service
        .authorize(request(), &接受提示)
        .await
        .expect("完整设备授权应成功");
    let secrets = SecureSecretFactory;
    let expected_message = canonical_device_registration_message(
        &secrets.digest(ASSERTION),
        "Windows 开发工作站",
        DevicePlatform::Windows,
        &DevicePublicSigningKey::new(vec![9; 32]).expect("测试公钥有效"),
    );
    let registrations = control_plane.registrations.lock().expect("注册锁未中毒");
    let stored = vault
        .value
        .lock()
        .expect("凭据锁未中毒")
        .clone()
        .expect("设备凭据必须写入安全存储");

    assert_eq!(
        *messages.lock().expect("签名消息锁未中毒"),
        vec![expected_message]
    );
    assert_eq!(registrations.len(), 1);
    assert_eq!(registrations[0].oidc_assertion.expose(), ASSERTION);
    assert!(registrations[0].import_display_name);
    assert!(registrations[0].import_locale);
    assert_eq!(stored.device_id, authorized.device_id);
    assert_eq!(stored.refresh_token.expose(), "bridge-refresh-token");
}

#[tokio::test]
async fn 控制平面已注册但安全存储失败时不得伪装授权成功() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let vault = Arc::new(内存凭据库 {
        value: Mutex::new(None),
        reject_writes: true,
    });
    let service = service(messages, Arc::new(测试控制平面::default()), vault);

    let failure = service
        .authorize(request(), &接受提示)
        .await
        .expect_err("安全存储失败必须向上返回");

    assert_eq!(
        failure.kind(),
        BridgeAuthorizationFailureKind::SecureStorageUnavailable
    );
    assert_eq!(failure.operation(), "bridge.authorize.persist_credentials");
}

fn service(
    messages: Arc<Mutex<Vec<Vec<u8>>>>,
    control_plane: Arc<测试控制平面>,
    vault: Arc<内存凭据库>,
) -> BridgeAuthorizationService {
    BridgeAuthorizationService::new(BridgeAuthorizationDependencies {
        oidc: Arc::new(测试Oidc),
        signing_identities: Arc::new(测试签名存储 {
            identity: Arc::new(测试签名身份 { messages }),
        }),
        control_plane,
        credentials: vault,
        secrets: Arc::new(SecureSecretFactory),
    })
}

fn request() -> AuthorizeBridgeDevice {
    AuthorizeBridgeDevice {
        label: "Windows 开发工作站".to_owned(),
        platform: DevicePlatform::Windows,
        profile_import: ProfileImportConsent {
            display_name: true,
            locale: true,
        },
    }
}

fn credentials() -> DeviceCredentials {
    let access_expires = UtcMillis::new(301_000).expect("测试时间有效");
    let refresh_expires = UtcMillis::new(86_401_000).expect("测试时间有效");
    let device_id = DeviceId::from_uuid(Uuid::from_u128(2));
    DeviceCredentials {
        device: AuthenticatedDevice {
            account: PrincipalAccount {
                principal: Principal::new(PrincipalId::from_uuid(Uuid::from_u128(1))),
                matrix_user_id: "@device-user:matrix.example".to_owned(),
                display_name: "设备用户".to_owned(),
                avatar_content_id: None,
                locale: "zh-CN".to_owned(),
            },
            device_id,
            access_token_expires_at: access_expires,
        },
        access_token: SecretValue::new("bridge-access-token").expect("测试 Token 有效"),
        refresh_token: SecretValue::new("bridge-refresh-token").expect("测试 Token 有效"),
        refresh_token_expires_at: refresh_expires,
    }
}
