//! 这台电脑的授权跟着账户登录走（`specs/desktop-sign-in/design.md`）。
//!
//! 桌面端登录着就能要一份一次性的电脑授权，原生层交给 Bridge，Bridge 拿它注册这台电脑，
//! 不用再批准设备码。授权只存摘要，10 分钟内有效，用一次就删；注册出来的令牌组一年上限
//! 从那份桌面登录真正认证的时刻算，不从注册的时刻算。

use std::sync::Arc;

use agent_room_domain::{
    devices::{Device, DevicePlatform, DevicePublicSigningKey, DeviceTokenFamily},
    time::UtcMillis,
};

use super::{
    DeviceAuthorizationFailureKind, DeviceAuthorizationPolicy, DeviceAuthorizationResult,
    DeviceAuthorizationService, DeviceCredentials, canonical_device_registration_message,
    credentials, failure, generate_secret, internal_failure, map_repository_failure,
    new_device_session, validate_device_label,
};
use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryErrorKind,
    ports::{
        Clock, ComputerGrantRecord, ComputerGrantStore, DeviceProofVerifier, DeviceSignature,
        IdentifierFactory, PortFuture, SecretFactory, SecretValue,
    },
};

const ISSUE_OPERATION: &str = "device.computer_grant.issue";
const REGISTER_OPERATION: &str = "device.register_with_grant";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedComputerGrant {
    pub grant: SecretValue,
    pub expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterDeviceWithGrant {
    pub grant: SecretValue,
    pub label: String,
    pub platform: DevicePlatform,
    pub public_signing_key: DevicePublicSigningKey,
    /// 签的是授权的摘要，消息格式和设备码那条路签 ID Token 摘要的一样。
    pub possession_signature: DeviceSignature,
}

pub trait ComputerGrantUseCases: Send + Sync {
    fn issue_computer_grant<'a>(
        &'a self,
        principal: &'a AuthenticatedPrincipal,
    ) -> PortFuture<'a, DeviceAuthorizationResult<IssuedComputerGrant>>;

    fn register_device_with_grant(
        &self,
        request: RegisterDeviceWithGrant,
    ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>>;
}

pub struct ComputerGrantService {
    grants: Arc<dyn ComputerGrantStore>,
    proof_verifier: Arc<dyn DeviceProofVerifier>,
    secrets: Arc<dyn SecretFactory>,
    identifiers: Arc<dyn IdentifierFactory>,
    clock: Arc<dyn Clock>,
    policy: DeviceAuthorizationPolicy,
}

impl DeviceAuthorizationService {
    /// 电脑授权和设备码注册用同一套时限、签名校验和令牌规则。
    pub fn computer_grants(&self, grants: Arc<dyn ComputerGrantStore>) -> ComputerGrantService {
        ComputerGrantService {
            grants,
            proof_verifier: self.proof_verifier.clone(),
            secrets: self.secrets.clone(),
            identifiers: self.identifiers.clone(),
            clock: self.clock.clone(),
            policy: self.policy.clone(),
        }
    }
}

impl ComputerGrantService {
    async fn issue(
        &self,
        principal: &AuthenticatedPrincipal,
    ) -> DeviceAuthorizationResult<IssuedComputerGrant> {
        let now = self.clock.now();
        let grant = generate_secret(&*self.secrets, ISSUE_OPERATION)?;
        let expires_at = now
            .checked_add(self.policy.device_authorization_maximum_age)
            .map_err(|_| internal_failure(ISSUE_OPERATION))?;
        let record = ComputerGrantRecord {
            grant_digest: self.secrets.digest(grant.expose()),
            principal_id: principal.principal_id,
            // 时钟回拨时认证时刻也不晚于签发时刻。
            authenticated_at: principal.authenticated_at.min(now),
            created_at: now,
            expires_at,
        };
        self.grants
            .insert_grant(&record)
            .await
            .map_err(|error| map_repository_failure(ISSUE_OPERATION, &error))?;
        Ok(IssuedComputerGrant { grant, expires_at })
    }

    async fn register(
        &self,
        request: RegisterDeviceWithGrant,
    ) -> DeviceAuthorizationResult<DeviceCredentials> {
        validate_device_label(&request.label)?;
        if request.platform == DevicePlatform::Network {
            return Err(failure(
                REGISTER_OPERATION,
                DeviceAuthorizationFailureKind::InvalidRequest,
            ));
        }
        let now = self.clock.now();
        let grant_digest = self.secrets.digest(request.grant.expose());
        let message = canonical_device_registration_message(
            &grant_digest,
            &request.label,
            request.platform,
            &request.public_signing_key,
        );
        if !self.proof_verifier.verify(
            &request.public_signing_key,
            &message,
            &request.possession_signature,
        ) {
            return Err(failure(
                REGISTER_OPERATION,
                DeviceAuthorizationFailureKind::InvalidProof,
            ));
        }
        let grant = self
            .grants
            .find_grant(&grant_digest, now)
            .await
            .map_err(|error| map_repository_failure(REGISTER_OPERATION, &error))?
            .ok_or_else(invalid_grant)?;
        let refresh_token_expires_at = self
            .policy
            .refresh_lifetime
            .renewed_until(grant.authenticated_at, now)
            .map_err(|_| internal_failure(REGISTER_OPERATION))?;
        if refresh_token_expires_at <= now {
            // 那份桌面登录已经满了一年，换不出新的电脑授权。
            return Err(invalid_grant());
        }
        let mut device = Device::register(
            self.identifiers.device_id(),
            grant.principal_id,
            request.label,
            request.platform,
            request.public_signing_key,
            now,
        )
        .map_err(|_| internal_failure(REGISTER_OPERATION))?;
        device
            .verify()
            .map_err(|_| internal_failure(REGISTER_OPERATION))?;
        let family = DeviceTokenFamily::new(
            self.identifiers.device_token_family_id(),
            device.id(),
            now,
            refresh_token_expires_at,
        )
        .and_then(|family| family.with_authenticated_at(grant.authenticated_at))
        .map_err(|_| internal_failure(REGISTER_OPERATION))?;
        let issued = new_device_session(
            &self.policy,
            &*self.identifiers,
            &*self.secrets,
            grant_digest,
            family,
            now,
            REGISTER_OPERATION,
        )?;
        let session = self
            .grants
            .register_with_grant(&grant, &device, &issued.registration)
            .await
            .map_err(|error| match error.kind() {
                // 同一份授权被并发用掉了一次。
                RepositoryErrorKind::NotFound => invalid_grant(),
                _ => map_repository_failure(REGISTER_OPERATION, &error),
            })?;
        Ok(credentials(
            &session,
            issued.access_token,
            issued.refresh_token,
            refresh_token_expires_at,
        ))
    }
}

impl ComputerGrantUseCases for ComputerGrantService {
    fn issue_computer_grant<'a>(
        &'a self,
        principal: &'a AuthenticatedPrincipal,
    ) -> PortFuture<'a, DeviceAuthorizationResult<IssuedComputerGrant>> {
        Box::pin(self.issue(principal))
    }

    fn register_device_with_grant(
        &self,
        request: RegisterDeviceWithGrant,
    ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>> {
        Box::pin(self.register(request))
    }
}

const fn invalid_grant() -> super::DeviceAuthorizationFailure {
    failure(
        REGISTER_OPERATION,
        DeviceAuthorizationFailureKind::InvalidAuthorization,
    )
}
