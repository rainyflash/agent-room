use std::sync::Arc;

use agent_room_application::{
    devices::canonical_device_registration_message,
    ports::{
        OidcDeviceAuthorizationPromptSink, OidcDeviceGrantGateway, OidcFailureKind,
        ProfileImportConsent, SecretFactory, SecretValue,
    },
};
use agent_room_domain::{
    devices::{DevicePlatform, DevicePublicSigningKey},
    ids::DeviceId,
    time::UtcMillis,
};

use crate::ports::{
    BridgeCredentialFailure, BridgeCredentialFailureKind, BridgeCredentialState,
    BridgeDeviceAuthorization, ControlPlaneDeviceFailure, ControlPlaneDeviceFailureKind,
    ControlPlaneDeviceGateway, DeviceCredentialVault, DeviceSigningIdentity,
    DeviceSigningIdentityStore, RegisterBridgeDevice, StoredBridgeDeviceCredentials,
};

const MAX_DEVICE_LABEL_LENGTH: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizeBridgeDevice {
    pub label: String,
    pub platform: DevicePlatform,
    pub profile_import: ProfileImportConsent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizedBridgeDevice {
    pub device_id: DeviceId,
    pub access_token_expires_at: UtcMillis,
    pub refresh_token_expires_at: UtcMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeAuthorizationFailureKind {
    InvalidRequest,
    AuthorizationDenied,
    AuthorizationExpired,
    AuthorizationPromptUnavailable,
    IdentityProviderUnavailable,
    InvalidIdentityAssertion,
    SecureStorageUnavailable,
    CorruptSecureStorage,
    ControlPlaneConflict,
    ControlPlaneUnavailable,
    UnknownCommit,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeAuthorizationFailure {
    operation: &'static str,
    kind: BridgeAuthorizationFailureKind,
}

impl BridgeAuthorizationFailure {
    const fn new(operation: &'static str, kind: BridgeAuthorizationFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> BridgeAuthorizationFailureKind {
        self.kind
    }
}

pub type BridgeAuthorizationResult<T> = Result<T, BridgeAuthorizationFailure>;

pub struct BridgeAuthorizationService {
    oidc: Arc<dyn OidcDeviceGrantGateway>,
    signing_identities: Arc<dyn DeviceSigningIdentityStore>,
    control_plane: Arc<dyn ControlPlaneDeviceGateway>,
    credentials: Arc<dyn DeviceCredentialVault>,
    secrets: Arc<dyn SecretFactory>,
    owner: Option<Arc<dyn crate::ports::BridgeOwnerRecord>>,
}

pub struct BridgeAuthorizationDependencies {
    pub oidc: Arc<dyn OidcDeviceGrantGateway>,
    pub signing_identities: Arc<dyn DeviceSigningIdentityStore>,
    pub control_plane: Arc<dyn ControlPlaneDeviceGateway>,
    pub credentials: Arc<dyn DeviceCredentialVault>,
    pub secrets: Arc<dyn SecretFactory>,
}

impl BridgeAuthorizationService {
    pub fn new(dependencies: BridgeAuthorizationDependencies) -> Self {
        Self {
            oidc: dependencies.oidc,
            signing_identities: dependencies.signing_identities,
            control_plane: dependencies.control_plane,
            credentials: dependencies.credentials,
            secrets: dependencies.secrets,
            owner: None,
        }
    }

    /// 授权成功时记下主人是谁（等消息时主人说话总能叫醒 Agent）。
    #[must_use]
    pub fn with_owner_record(mut self, owner: Arc<dyn crate::ports::BridgeOwnerRecord>) -> Self {
        self.owner = Some(owner);
        self
    }

    /// 完成人类设备授权、设备持有证明、控制平面注册和凭据落库。
    ///
    /// # Errors
    ///
    /// 输入非法、OIDC 被拒绝、依赖不可用、签名失败或 OS 安全存储失败时返回稳定错误。
    pub async fn authorize(
        &self,
        request: AuthorizeBridgeDevice,
        prompt_sink: &dyn OidcDeviceAuthorizationPromptSink,
    ) -> BridgeAuthorizationResult<AuthorizedBridgeDevice> {
        validate_label(&request.label)?;
        // 先确认本机钥匙可用，再给用户看设备码：批准完才发现存不了钥匙，这次批准就白费了。
        let signing = self.signing_key()?;
        let assertion = self.oidc.authorize(prompt_sink).await.map_err(|error| {
            let kind = match error.kind() {
                OidcFailureKind::DependencyUnavailable => {
                    BridgeAuthorizationFailureKind::IdentityProviderUnavailable
                }
                OidcFailureKind::ProviderRejected => {
                    BridgeAuthorizationFailureKind::AuthorizationDenied
                }
                OidcFailureKind::AuthorizationExpired => {
                    BridgeAuthorizationFailureKind::AuthorizationExpired
                }
                OidcFailureKind::PromptUnavailable => {
                    BridgeAuthorizationFailureKind::AuthorizationPromptUnavailable
                }
                OidcFailureKind::InvalidIdentityToken => {
                    BridgeAuthorizationFailureKind::InvalidIdentityAssertion
                }
                OidcFailureKind::InvalidConfiguration => BridgeAuthorizationFailureKind::Internal,
            };
            failure("bridge.authorize.oidc", kind)
        })?;
        self.register_and_store(
            request,
            signing,
            BridgeDeviceAuthorization::OidcAssertion(assertion),
        )
        .await
    }

    /// 拿桌面端交来的一次性电脑授权注册这台电脑，不用设备码（`specs/desktop-sign-in/design.md`）。
    ///
    /// # Errors
    ///
    /// 授权用过、过期或被拒时返回 `InvalidIdentityAssertion`；其余同 [`Self::authorize`]。
    pub async fn authorize_with_grant(
        &self,
        request: AuthorizeBridgeDevice,
        grant: SecretValue,
    ) -> BridgeAuthorizationResult<AuthorizedBridgeDevice> {
        validate_label(&request.label)?;
        let signing = self.signing_key()?;
        self.register_and_store(
            request,
            signing,
            BridgeDeviceAuthorization::ComputerGrant(grant),
        )
        .await
    }

    fn signing_key(
        &self,
    ) -> BridgeAuthorizationResult<(Arc<dyn DeviceSigningIdentity>, DevicePublicSigningKey)> {
        let signing_identity = self
            .signing_identities
            .load_or_create()
            .map_err(|error| map_credential_failure("bridge.authorize.load_key", error))?;
        let public_signing_key = signing_identity
            .public_key()
            .map_err(|error| map_credential_failure("bridge.authorize.public_key", error))?;
        Ok((signing_identity, public_signing_key))
    }

    /// 用这台设备的签名钥匙签下授权的摘要，向控制面注册，存好拿回来的凭据。
    async fn register_and_store(
        &self,
        request: AuthorizeBridgeDevice,
        (signing_identity, public_signing_key): (
            Arc<dyn DeviceSigningIdentity>,
            DevicePublicSigningKey,
        ),
        authorization: BridgeDeviceAuthorization,
    ) -> BridgeAuthorizationResult<AuthorizedBridgeDevice> {
        let authorization_digest = self.secrets.digest(authorization.secret().expose());
        let registration_message = canonical_device_registration_message(
            &authorization_digest,
            &request.label,
            request.platform,
            &public_signing_key,
        );
        let possession_signature = signing_identity
            .sign(&registration_message)
            .map_err(|error| map_credential_failure("bridge.authorize.sign", error))?;
        let credentials = self
            .control_plane
            .register(RegisterBridgeDevice {
                authorization,
                label: request.label,
                platform: request.platform,
                public_signing_key,
                possession_signature,
                import_display_name: request.profile_import.display_name,
                import_locale: request.profile_import.locale,
            })
            .await
            .map_err(map_control_plane_failure)?;
        if let Some(owner) = &self.owner {
            owner.remember(&crate::ports::BridgeOwner {
                principal_id: credentials.device.account.principal.id().to_string(),
                matrix_user_id: credentials.device.account.matrix_user_id.clone(),
            });
        }
        let stored = StoredBridgeDeviceCredentials {
            state: BridgeCredentialState::Ready,
            device_id: credentials.device.device_id,
            access_token: credentials.access_token,
            access_token_expires_at: credentials.device.access_token_expires_at,
            refresh_token: credentials.refresh_token,
            refresh_token_expires_at: credentials.refresh_token_expires_at,
        };
        self.credentials.replace(&stored).map_err(|error| {
            map_credential_failure("bridge.authorize.persist_credentials", error)
        })?;

        Ok(AuthorizedBridgeDevice {
            device_id: stored.device_id,
            access_token_expires_at: stored.access_token_expires_at,
            refresh_token_expires_at: stored.refresh_token_expires_at,
        })
    }
}

fn validate_label(label: &str) -> BridgeAuthorizationResult<()> {
    if label.is_empty()
        || label.len() > MAX_DEVICE_LABEL_LENGTH
        || label.chars().any(char::is_control)
    {
        return Err(failure(
            "bridge.authorize.validate",
            BridgeAuthorizationFailureKind::InvalidRequest,
        ));
    }
    Ok(())
}

const fn map_credential_failure(
    operation: &'static str,
    error: BridgeCredentialFailure,
) -> BridgeAuthorizationFailure {
    let kind = match error.kind() {
        BridgeCredentialFailureKind::Unavailable | BridgeCredentialFailureKind::StorageFull => {
            BridgeAuthorizationFailureKind::SecureStorageUnavailable
        }
        BridgeCredentialFailureKind::Corrupt => {
            BridgeAuthorizationFailureKind::CorruptSecureStorage
        }
    };
    failure(operation, kind)
}

const fn map_control_plane_failure(error: ControlPlaneDeviceFailure) -> BridgeAuthorizationFailure {
    let kind = match error.kind() {
        ControlPlaneDeviceFailureKind::InvalidRequest
        | ControlPlaneDeviceFailureKind::AuthenticationRejected => {
            BridgeAuthorizationFailureKind::InvalidIdentityAssertion
        }
        ControlPlaneDeviceFailureKind::Conflict => {
            BridgeAuthorizationFailureKind::ControlPlaneConflict
        }
        ControlPlaneDeviceFailureKind::DependencyUnavailable => {
            BridgeAuthorizationFailureKind::ControlPlaneUnavailable
        }
        ControlPlaneDeviceFailureKind::UnknownCommit => {
            BridgeAuthorizationFailureKind::UnknownCommit
        }
        ControlPlaneDeviceFailureKind::Internal => BridgeAuthorizationFailureKind::Internal,
    };
    failure("bridge.authorize.register", kind)
}

const fn failure(
    operation: &'static str,
    kind: BridgeAuthorizationFailureKind,
) -> BridgeAuthorizationFailure {
    BridgeAuthorizationFailure::new(operation, kind)
}
