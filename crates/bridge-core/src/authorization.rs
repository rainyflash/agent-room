use std::sync::Arc;

use agent_room_application::{
    devices::canonical_device_registration_message,
    ports::{
        OidcDeviceAuthorizationPromptSink, OidcDeviceGrantGateway, OidcFailureKind,
        ProfileImportConsent, SecretFactory,
    },
};
use agent_room_domain::{devices::DevicePlatform, ids::DeviceId, time::UtcMillis};

use crate::ports::{
    BridgeCredentialFailure, BridgeCredentialFailureKind, BridgeCredentialState,
    ControlPlaneDeviceFailure, ControlPlaneDeviceFailureKind, ControlPlaneDeviceGateway,
    DeviceCredentialVault, DeviceSigningIdentityStore, RegisterBridgeDevice,
    StoredBridgeDeviceCredentials,
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
        }
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
        let signing_identity = self
            .signing_identities
            .load_or_create()
            .map_err(|error| map_credential_failure("bridge.authorize.load_key", error))?;
        let public_signing_key = signing_identity
            .public_key()
            .map_err(|error| map_credential_failure("bridge.authorize.public_key", error))?;
        let assertion = self.oidc.authorize(prompt_sink).await.map_err(|error| {
            let kind = match error.kind() {
                OidcFailureKind::DependencyUnavailable => {
                    BridgeAuthorizationFailureKind::IdentityProviderUnavailable
                }
                OidcFailureKind::ProviderRejected => {
                    BridgeAuthorizationFailureKind::AuthorizationDenied
                }
                OidcFailureKind::InvalidIdentityToken => {
                    BridgeAuthorizationFailureKind::InvalidIdentityAssertion
                }
                OidcFailureKind::InvalidConfiguration => BridgeAuthorizationFailureKind::Internal,
            };
            failure("bridge.authorize.oidc", kind)
        })?;
        let assertion_digest = self.secrets.digest(assertion.expose());
        let registration_message = canonical_device_registration_message(
            &assertion_digest,
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
                oidc_assertion: assertion,
                label: request.label,
                platform: request.platform,
                public_signing_key,
                possession_signature,
                import_display_name: request.profile_import.display_name,
                import_locale: request.profile_import.locale,
            })
            .await
            .map_err(map_control_plane_failure)?;
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
        BridgeCredentialFailureKind::Unavailable => {
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
