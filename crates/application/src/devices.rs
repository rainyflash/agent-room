use std::sync::Arc;

use agent_room_domain::{
    devices::{
        Device, DevicePlatform, DevicePublicSigningKey, DeviceTokenFamily, DeviceTrustState,
    },
    ids::{DeviceId, PrincipalId},
    time::{DurationMillis, UtcMillis},
};

use crate::{
    matrix_device_cleanup::revoke_agent_matrix_device,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        AgentInstanceMatrixCleanupStore, Clock, DeviceProofNonceStore, DeviceProofVerifier,
        DeviceRefreshOutcome, DeviceRegistrationTransaction, DeviceRepository,
        DeviceRevocationOutcome, DeviceRevocationTransaction, DeviceSecurityEvent,
        DeviceSessionRegistration, DeviceSessionStore, DeviceSignature, DeviceTokenReplacement,
        IdentifierFactory, MatrixAgentDeviceSessionRevoker, PendingAgentMatrixDeviceRevocation,
        PortFuture, PrincipalAccount, ProfileImportConsent, SecretDigest, SecretFactory,
        SecretValue, StoredDeviceSession, VerifiedOidcIdentity,
    },
};

const MAX_DEVICE_LABEL_LENGTH: usize = 128;
const MAX_REQUEST_METHOD_LENGTH: usize = 16;
const MAX_REQUEST_TARGET_LENGTH: usize = 2_048;
const MIN_PROOF_NONCE_LENGTH: usize = 16;
const MAX_PROOF_NONCE_LENGTH: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAuthorizationPolicy {
    access_token_ttl: DurationMillis,
    refresh_token_ttl: DurationMillis,
    proof_maximum_age: DurationMillis,
    allowed_clock_skew: DurationMillis,
    device_authorization_maximum_age: DurationMillis,
    matrix_server_name: String,
}

impl DeviceAuthorizationPolicy {
    /// 创建设备授权、Token 和签名证明策略。
    ///
    /// # Errors
    ///
    /// 生命周期顺序或 Matrix 服务名不满足安全边界时返回配置错误。
    pub fn new(
        access_token_ttl: DurationMillis,
        refresh_token_ttl: DurationMillis,
        proof_maximum_age: DurationMillis,
        allowed_clock_skew: DurationMillis,
        device_authorization_maximum_age: DurationMillis,
        matrix_server_name: impl Into<String>,
    ) -> Result<Self, DeviceAuthorizationConfigurationError> {
        let matrix_server_name = matrix_server_name.into();
        if access_token_ttl >= refresh_token_ttl {
            return Err(DeviceAuthorizationConfigurationError::InvalidTokenLifetimes);
        }
        if matrix_server_name.is_empty()
            || matrix_server_name.len() > 255
            || matrix_server_name.chars().any(char::is_control)
            || matrix_server_name.contains(['/', '\\', '@'])
        {
            return Err(DeviceAuthorizationConfigurationError::InvalidMatrixServerName);
        }
        Ok(Self {
            access_token_ttl,
            refresh_token_ttl,
            proof_maximum_age,
            allowed_clock_skew,
            device_authorization_maximum_age,
            matrix_server_name,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceAuthorizationConfigurationError {
    InvalidTokenLifetimes,
    InvalidMatrixServerName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedDeviceAuthorization {
    identity: VerifiedOidcIdentity,
    authorization_token_digest: SecretDigest,
}

impl VerifiedDeviceAuthorization {
    /// 仅供已经完成 OIDC 签名、issuer、audience 和期限校验的边界适配器调用。
    pub const fn new(
        identity: VerifiedOidcIdentity,
        authorization_token_digest: SecretDigest,
    ) -> Self {
        Self {
            identity,
            authorization_token_digest,
        }
    }

    pub const fn identity(&self) -> &VerifiedOidcIdentity {
        &self.identity
    }

    pub const fn authorization_token_digest(&self) -> &SecretDigest {
        &self.authorization_token_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterDevice {
    pub authorization: VerifiedDeviceAuthorization,
    pub label: String,
    pub platform: DevicePlatform,
    pub public_signing_key: DevicePublicSigningKey,
    pub possession_signature: DeviceSignature,
    pub profile_import: ProfileImportConsent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRequestProofPayload {
    device_id: DeviceId,
    issued_at: UtcMillis,
    nonce: SecretValue,
    method: String,
    request_target: String,
    body_digest: SecretDigest,
}

impl DeviceRequestProofPayload {
    /// 创建可由 Bridge 规范签名的请求载荷。
    ///
    /// # Errors
    ///
    /// 方法、请求目标或 nonce 不符合边界约束时返回无效请求。
    pub fn new(
        device_id: DeviceId,
        issued_at: UtcMillis,
        nonce: SecretValue,
        method: String,
        request_target: String,
        body_digest: SecretDigest,
    ) -> Result<Self, DeviceRequestProofError> {
        if method.is_empty()
            || method.len() > MAX_REQUEST_METHOD_LENGTH
            || !method.bytes().all(|byte| byte.is_ascii_uppercase())
        {
            return Err(DeviceRequestProofError::InvalidMethod);
        }
        if request_target.is_empty()
            || request_target.len() > MAX_REQUEST_TARGET_LENGTH
            || !request_target.starts_with('/')
            || request_target.starts_with("//")
            || request_target.contains(['\\', '#'])
            || request_target.chars().any(char::is_control)
        {
            return Err(DeviceRequestProofError::InvalidRequestTarget);
        }
        if !(MIN_PROOF_NONCE_LENGTH..=MAX_PROOF_NONCE_LENGTH).contains(&nonce.expose().len()) {
            return Err(DeviceRequestProofError::InvalidNonce);
        }
        Ok(Self {
            device_id,
            issued_at,
            nonce,
            method,
            request_target,
            body_digest,
        })
    }

    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    pub const fn issued_at(&self) -> UtcMillis {
        self.issued_at
    }

    pub const fn nonce(&self) -> &SecretValue {
        &self.nonce
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn request_target(&self) -> &str {
        &self.request_target
    }

    pub const fn body_digest(&self) -> &SecretDigest {
        &self.body_digest
    }

    pub fn signing_message(&self, credential_digest: &SecretDigest) -> Vec<u8> {
        canonical_device_request_message(self, credential_digest)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRequestProof {
    payload: DeviceRequestProofPayload,
    signature: DeviceSignature,
}

impl DeviceRequestProof {
    pub const fn new(payload: DeviceRequestProofPayload, signature: DeviceSignature) -> Self {
        Self { payload, signature }
    }

    pub const fn payload(&self) -> &DeviceRequestProofPayload {
        &self.payload
    }

    pub const fn device_id(&self) -> DeviceId {
        self.payload.device_id()
    }

    pub const fn issued_at(&self) -> UtcMillis {
        self.payload.issued_at()
    }

    pub const fn nonce(&self) -> &SecretValue {
        self.payload.nonce()
    }

    pub fn method(&self) -> &str {
        self.payload.method()
    }

    pub fn request_target(&self) -> &str {
        self.payload.request_target()
    }

    pub const fn body_digest(&self) -> &SecretDigest {
        self.payload.body_digest()
    }

    pub const fn signature(&self) -> &DeviceSignature {
        &self.signature
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceRequestProofError {
    InvalidMethod,
    InvalidRequestTarget,
    InvalidNonce,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticateDeviceRequest<'a> {
    pub access_token: &'a SecretValue,
    pub proof: &'a DeviceRequestProof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshDeviceSession<'a> {
    pub refresh_token: &'a SecretValue,
    pub proof: &'a DeviceRequestProof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedDevice {
    pub account: PrincipalAccount,
    pub device_id: DeviceId,
    pub access_token_expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCredentials {
    pub device: AuthenticatedDevice,
    pub access_token: SecretValue,
    pub refresh_token: SecretValue,
    pub refresh_token_expires_at: UtcMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceAuthorizationFailureKind {
    InvalidRequest,
    InvalidAuthorization,
    InvalidToken,
    InvalidProof,
    ProofReplay,
    RefreshTokenReuse,
    PrincipalSuspended,
    DeviceRevoked,
    NotFound,
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceAuthorizationFailure {
    operation: &'static str,
    kind: DeviceAuthorizationFailureKind,
}

impl DeviceAuthorizationFailure {
    const fn new(operation: &'static str, kind: DeviceAuthorizationFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> DeviceAuthorizationFailureKind {
        self.kind
    }
}

pub type DeviceAuthorizationResult<T> = Result<T, DeviceAuthorizationFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceMatrixCleanup {
    Complete,
    Pending { agent_instance_count: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevokedDevice {
    pub matrix_cleanup: DeviceMatrixCleanup,
}

pub trait DeviceAuthorizationUseCases: Send + Sync {
    fn register_device(
        &self,
        request: RegisterDevice,
    ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>>;

    fn authenticate_device<'a>(
        &'a self,
        request: AuthenticateDeviceRequest<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<AuthenticatedDevice>>;

    fn refresh_device_session<'a>(
        &'a self,
        request: RefreshDeviceSession<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<DeviceCredentials>>;

    fn list_devices(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<Vec<Device>>>;

    fn revoke_device(
        &self,
        principal_id: PrincipalId,
        device_id: DeviceId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<RevokedDevice>>;
}

pub struct DeviceAuthorizationService {
    registrations: Arc<dyn DeviceRegistrationTransaction>,
    sessions: Arc<dyn DeviceSessionStore>,
    proof_nonces: Arc<dyn DeviceProofNonceStore>,
    proof_verifier: Arc<dyn DeviceProofVerifier>,
    devices: Arc<dyn DeviceRepository>,
    revocations: Arc<dyn DeviceRevocationTransaction>,
    matrix_cleanup: Arc<dyn AgentInstanceMatrixCleanupStore>,
    matrix: Arc<dyn MatrixAgentDeviceSessionRevoker>,
    secrets: Arc<dyn SecretFactory>,
    identifiers: Arc<dyn IdentifierFactory>,
    clock: Arc<dyn Clock>,
    policy: DeviceAuthorizationPolicy,
}

pub struct DeviceAuthorizationDependencies {
    pub registrations: Arc<dyn DeviceRegistrationTransaction>,
    pub sessions: Arc<dyn DeviceSessionStore>,
    pub proof_nonces: Arc<dyn DeviceProofNonceStore>,
    pub proof_verifier: Arc<dyn DeviceProofVerifier>,
    pub devices: Arc<dyn DeviceRepository>,
    pub revocations: Arc<dyn DeviceRevocationTransaction>,
    pub matrix_cleanup: Arc<dyn AgentInstanceMatrixCleanupStore>,
    pub matrix: Arc<dyn MatrixAgentDeviceSessionRevoker>,
    pub secrets: Arc<dyn SecretFactory>,
    pub identifiers: Arc<dyn IdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

impl DeviceAuthorizationService {
    pub fn new(
        dependencies: DeviceAuthorizationDependencies,
        policy: DeviceAuthorizationPolicy,
    ) -> Self {
        Self {
            registrations: dependencies.registrations,
            sessions: dependencies.sessions,
            proof_nonces: dependencies.proof_nonces,
            proof_verifier: dependencies.proof_verifier,
            devices: dependencies.devices,
            revocations: dependencies.revocations,
            matrix_cleanup: dependencies.matrix_cleanup,
            matrix: dependencies.matrix,
            secrets: dependencies.secrets,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
            policy,
        }
    }

    async fn register_device_internal(
        &self,
        request: RegisterDevice,
    ) -> DeviceAuthorizationResult<DeviceCredentials> {
        validate_device_label(&request.label)?;
        let now = self.clock.now();
        validate_device_authorization_time(request.authorization.identity(), now, &self.policy)?;
        let registration_message = canonical_device_registration_message(
            request.authorization.authorization_token_digest(),
            &request.label,
            request.platform,
            &request.public_signing_key,
        );
        if !self.proof_verifier.verify(
            &request.public_signing_key,
            &registration_message,
            &request.possession_signature,
        ) {
            return Err(failure(
                "device.register",
                DeviceAuthorizationFailureKind::InvalidProof,
            ));
        }

        let principal = crate::principal_projection::principal_registration(
            self.identifiers.principal_id(),
            request.authorization.identity(),
            request.profile_import,
            now,
            &self.policy.matrix_server_name,
        );
        let mut device = Device::register(
            self.identifiers.device_id(),
            principal.principal.id(),
            request.label,
            request.platform,
            request.public_signing_key,
            now,
        )
        .map_err(|_| internal_failure("device.register"))?;
        device
            .verify()
            .map_err(|_| internal_failure("device.register"))?;
        let access_token = generate_secret(&*self.secrets, "device.register")?;
        let refresh_token = generate_secret(&*self.secrets, "device.register")?;
        let access_token_expires_at = now
            .checked_add(self.policy.access_token_ttl)
            .map_err(|_| internal_failure("device.register"))?;
        let refresh_token_expires_at = now
            .checked_add(self.policy.refresh_token_ttl)
            .map_err(|_| internal_failure("device.register"))?;
        let family = DeviceTokenFamily::new(
            self.identifiers.device_token_family_id(),
            device.id(),
            now,
            refresh_token_expires_at,
        )
        .map_err(|_| internal_failure("device.register"))?;
        let session_registration = DeviceSessionRegistration {
            authorization_token_digest: *request.authorization.authorization_token_digest(),
            authorization_receipt_expires_at: now
                .checked_add(self.policy.device_authorization_maximum_age)
                .and_then(|value| value.checked_add(self.policy.allowed_clock_skew))
                .map_err(|_| internal_failure("device.register"))?,
            family,
            access_token_id: self.identifiers.device_access_token_id(),
            access_token_digest: self.secrets.digest(access_token.expose()),
            access_token_expires_at,
            refresh_token_id: self.identifiers.device_refresh_token_id(),
            refresh_token_digest: self.secrets.digest(refresh_token.expose()),
            issued_at: now,
        };
        let session = self
            .registrations
            .register(&principal, &device, &session_registration)
            .await
            .map_err(|error| map_repository_failure("device.register", &error))?;

        Ok(credentials(
            &session,
            access_token,
            refresh_token,
            refresh_token_expires_at,
        ))
    }

    async fn authenticate_device_internal(
        &self,
        request: AuthenticateDeviceRequest<'_>,
    ) -> DeviceAuthorizationResult<AuthenticatedDevice> {
        let now = self.clock.now();
        let access_digest = self.secrets.digest(request.access_token.expose());
        let session = self
            .sessions
            .find_active_access(&access_digest, now)
            .await
            .map_err(|error| map_repository_failure("device.authenticate", &error))?
            .ok_or_else(|| {
                failure(
                    "device.authenticate",
                    DeviceAuthorizationFailureKind::InvalidToken,
                )
            })?;
        ensure_session_active(&session, "device.authenticate")?;
        self.verify_request_proof(&session.device, request.proof, &access_digest, now)
            .await?;

        Ok(authenticated_device(&session))
    }

    async fn refresh_device_session_internal(
        &self,
        request: RefreshDeviceSession<'_>,
    ) -> DeviceAuthorizationResult<DeviceCredentials> {
        let operation = "device.refresh";
        let now = self.clock.now();
        let refresh_digest = self.secrets.digest(request.refresh_token.expose());
        let context = self
            .sessions
            .find_refresh_context(&refresh_digest)
            .await
            .map_err(|error| map_repository_failure(operation, &error))?
            .ok_or_else(|| failure(operation, DeviceAuthorizationFailureKind::InvalidToken))?;
        if !context.account.principal.allows_authentication() {
            return Err(failure(
                operation,
                DeviceAuthorizationFailureKind::PrincipalSuspended,
            ));
        }
        if !context.device.accepts_authenticated_requests() {
            return Err(failure(
                operation,
                DeviceAuthorizationFailureKind::DeviceRevoked,
            ));
        }
        if !context.family.allows_rotation(now) {
            return Err(failure(
                operation,
                DeviceAuthorizationFailureKind::InvalidToken,
            ));
        }
        self.verify_signature_only(&context.device, request.proof, &refresh_digest, now)?;

        let access_token = generate_secret(&*self.secrets, operation)?;
        let refresh_token = generate_secret(&*self.secrets, operation)?;
        let replacement = DeviceTokenReplacement {
            access_token_id: self.identifiers.device_access_token_id(),
            access_token_digest: self.secrets.digest(access_token.expose()),
            access_token_expires_at: now
                .checked_add(self.policy.access_token_ttl)
                .map_err(|_| internal_failure(operation))?,
            refresh_token_id: self.identifiers.device_refresh_token_id(),
            refresh_token_digest: self.secrets.digest(refresh_token.expose()),
            issued_at: now,
        };
        let outcome = self
            .sessions
            .rotate_refresh(
                &refresh_digest,
                &replacement,
                DeviceSecurityEvent {
                    id: self.identifiers.outbox_event_id(),
                    occurred_at: now,
                },
            )
            .await
            .map_err(|error| map_repository_failure(operation, &error))?;

        match outcome {
            DeviceRefreshOutcome::Rotated {
                session,
                refresh_token_expires_at,
            } => Ok(credentials(
                &session,
                access_token,
                refresh_token,
                refresh_token_expires_at,
            )),
            DeviceRefreshOutcome::ReuseDetected { .. } => Err(failure(
                operation,
                DeviceAuthorizationFailureKind::RefreshTokenReuse,
            )),
            DeviceRefreshOutcome::Rejected => Err(failure(
                operation,
                DeviceAuthorizationFailureKind::InvalidToken,
            )),
        }
    }

    async fn list_devices_internal(
        &self,
        principal_id: PrincipalId,
    ) -> DeviceAuthorizationResult<Vec<Device>> {
        self.devices
            .list_for_principal(principal_id)
            .await
            .map_err(|error| map_repository_failure("device.list", &error))
    }

    async fn revoke_device_internal(
        &self,
        principal_id: PrincipalId,
        device_id: DeviceId,
    ) -> DeviceAuthorizationResult<RevokedDevice> {
        let now = self.clock.now();
        let outcome = self
            .revocations
            .revoke(
                principal_id,
                device_id,
                DeviceSecurityEvent {
                    id: self.identifiers.outbox_event_id(),
                    occurred_at: now,
                },
            )
            .await
            .map_err(|error| map_repository_failure("device.revoke", &error))?;
        let pending = match outcome {
            DeviceRevocationOutcome::Revoked(pending)
            | DeviceRevocationOutcome::AlreadyRevoked(pending) => pending,
            DeviceRevocationOutcome::NotFound => Err(failure(
                "device.revoke",
                DeviceAuthorizationFailureKind::NotFound,
            ))?,
        };
        let pending_count = self.cleanup_matrix_devices(pending, now).await;
        let matrix_cleanup = if pending_count == 0 {
            DeviceMatrixCleanup::Complete
        } else {
            DeviceMatrixCleanup::Pending {
                agent_instance_count: pending_count,
            }
        };
        Ok(RevokedDevice { matrix_cleanup })
    }

    async fn cleanup_matrix_devices(
        &self,
        pending: Vec<PendingAgentMatrixDeviceRevocation>,
        revoked_at: UtcMillis,
    ) -> usize {
        let mut pending_count = 0;
        for target in pending {
            if revoke_agent_matrix_device(
                self.matrix.as_ref(),
                self.matrix_cleanup.as_ref(),
                target.instance_id,
                &target.matrix_user_id,
                &target.matrix_device_id,
                revoked_at,
            )
            .await
            .is_err()
            {
                pending_count += 1;
            }
        }
        pending_count
    }

    async fn verify_request_proof(
        &self,
        device: &Device,
        proof: &DeviceRequestProof,
        credential_digest: &SecretDigest,
        now: UtcMillis,
    ) -> DeviceAuthorizationResult<()> {
        self.verify_signature_only(device, proof, credential_digest, now)?;
        let expires_at = proof
            .issued_at()
            .checked_add(self.policy.proof_maximum_age)
            .and_then(|value| value.checked_add(self.policy.allowed_clock_skew))
            .map_err(|_| internal_failure("device.authenticate"))?;
        let consumed = self
            .proof_nonces
            .consume(
                device.id(),
                &self.secrets.digest(proof.nonce().expose()),
                now,
                expires_at,
            )
            .await
            .map_err(|error| map_repository_failure("device.authenticate", &error))?;
        if !consumed {
            return Err(failure(
                "device.authenticate",
                DeviceAuthorizationFailureKind::ProofReplay,
            ));
        }
        Ok(())
    }

    fn verify_signature_only(
        &self,
        device: &Device,
        proof: &DeviceRequestProof,
        credential_digest: &SecretDigest,
        now: UtcMillis,
    ) -> DeviceAuthorizationResult<()> {
        if proof.device_id() != device.id() {
            return Err(failure(
                "device.proof",
                DeviceAuthorizationFailureKind::InvalidProof,
            ));
        }
        validate_proof_time(proof.issued_at(), now, &self.policy)?;
        let message = proof.payload().signing_message(credential_digest);
        if !self
            .proof_verifier
            .verify(device.public_signing_key(), &message, proof.signature())
        {
            return Err(failure(
                "device.proof",
                DeviceAuthorizationFailureKind::InvalidProof,
            ));
        }
        Ok(())
    }
}

impl DeviceAuthorizationUseCases for DeviceAuthorizationService {
    fn register_device(
        &self,
        request: RegisterDevice,
    ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>> {
        Box::pin(self.register_device_internal(request))
    }

    fn authenticate_device<'a>(
        &'a self,
        request: AuthenticateDeviceRequest<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<AuthenticatedDevice>> {
        Box::pin(self.authenticate_device_internal(request))
    }

    fn refresh_device_session<'a>(
        &'a self,
        request: RefreshDeviceSession<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<DeviceCredentials>> {
        Box::pin(self.refresh_device_session_internal(request))
    }

    fn list_devices(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<Vec<Device>>> {
        Box::pin(self.list_devices_internal(principal_id))
    }

    fn revoke_device(
        &self,
        principal_id: PrincipalId,
        device_id: DeviceId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<RevokedDevice>> {
        Box::pin(self.revoke_device_internal(principal_id, device_id))
    }
}

fn ensure_session_active(
    session: &StoredDeviceSession,
    operation: &'static str,
) -> DeviceAuthorizationResult<()> {
    if !session.account.principal.allows_authentication() {
        return Err(failure(
            operation,
            DeviceAuthorizationFailureKind::PrincipalSuspended,
        ));
    }
    if session.device.trust_state() != DeviceTrustState::Verified {
        return Err(failure(
            operation,
            DeviceAuthorizationFailureKind::DeviceRevoked,
        ));
    }
    Ok(())
}

fn validate_device_label(label: &str) -> DeviceAuthorizationResult<()> {
    if label.is_empty()
        || label.len() > MAX_DEVICE_LABEL_LENGTH
        || label.chars().any(char::is_control)
    {
        return Err(failure(
            "device.register",
            DeviceAuthorizationFailureKind::InvalidRequest,
        ));
    }
    Ok(())
}

fn validate_device_authorization_time(
    identity: &VerifiedOidcIdentity,
    now: UtcMillis,
    policy: &DeviceAuthorizationPolicy,
) -> DeviceAuthorizationResult<()> {
    let authenticated_at = identity.authenticated_at().ok_or_else(|| {
        failure(
            "device.register",
            DeviceAuthorizationFailureKind::InvalidAuthorization,
        )
    })?;
    let latest_allowed = now
        .checked_add(policy.allowed_clock_skew)
        .map_err(|_| internal_failure("device.register"))?;
    let oldest_allowed = authenticated_at
        .checked_add(policy.device_authorization_maximum_age)
        .and_then(|value| value.checked_add(policy.allowed_clock_skew))
        .map_err(|_| internal_failure("device.register"))?;
    if authenticated_at > latest_allowed || now > oldest_allowed {
        return Err(failure(
            "device.register",
            DeviceAuthorizationFailureKind::InvalidAuthorization,
        ));
    }
    Ok(())
}

fn validate_proof_time(
    issued_at: UtcMillis,
    now: UtcMillis,
    policy: &DeviceAuthorizationPolicy,
) -> DeviceAuthorizationResult<()> {
    let latest_allowed = now
        .checked_add(policy.allowed_clock_skew)
        .map_err(|_| internal_failure("device.proof"))?;
    let expires_at = issued_at
        .checked_add(policy.proof_maximum_age)
        .and_then(|value| value.checked_add(policy.allowed_clock_skew))
        .map_err(|_| internal_failure("device.proof"))?;
    if issued_at > latest_allowed || now > expires_at {
        return Err(failure(
            "device.proof",
            DeviceAuthorizationFailureKind::InvalidProof,
        ));
    }
    Ok(())
}

pub fn canonical_device_registration_message(
    authorization_token_digest: &SecretDigest,
    label: &str,
    platform: DevicePlatform,
    public_key: &DevicePublicSigningKey,
) -> Vec<u8> {
    format!(
        "agent-room-device-registration-v1\n{}\n{}\n{}\n{}",
        encode_hex(authorization_token_digest.as_bytes()),
        label,
        platform.as_str(),
        encode_hex(public_key.as_bytes())
    )
    .into_bytes()
}

fn canonical_device_request_message(
    proof: &DeviceRequestProofPayload,
    credential_digest: &SecretDigest,
) -> Vec<u8> {
    format!(
        "agent-room-device-request-v1\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        proof.device_id(),
        proof.method(),
        proof.request_target(),
        encode_hex(proof.body_digest().as_bytes()),
        proof.issued_at().value(),
        proof.nonce().expose(),
        encode_hex(credential_digest.as_bytes())
    )
    .into_bytes()
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("写入 String 不会失败");
    }
    encoded
}

fn generate_secret(
    factory: &dyn SecretFactory,
    operation: &'static str,
) -> DeviceAuthorizationResult<SecretValue> {
    factory.generate().map_err(|_| internal_failure(operation))
}

fn authenticated_device(session: &StoredDeviceSession) -> AuthenticatedDevice {
    AuthenticatedDevice {
        account: session.account.clone(),
        device_id: session.device.id(),
        access_token_expires_at: session.access_token_expires_at,
    }
}

fn credentials(
    session: &StoredDeviceSession,
    access_token: SecretValue,
    refresh_token: SecretValue,
    refresh_token_expires_at: UtcMillis,
) -> DeviceCredentials {
    let device = authenticated_device(session);
    DeviceCredentials {
        device,
        access_token,
        refresh_token,
        refresh_token_expires_at,
    }
}

const fn map_repository_failure(
    operation: &'static str,
    error: &RepositoryError,
) -> DeviceAuthorizationFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Conflict => DeviceAuthorizationFailureKind::Conflict,
        RepositoryErrorKind::Forbidden => DeviceAuthorizationFailureKind::DeviceRevoked,
        RepositoryErrorKind::NotFound => DeviceAuthorizationFailureKind::NotFound,
        RepositoryErrorKind::Unavailable => DeviceAuthorizationFailureKind::DependencyUnavailable,
        RepositoryErrorKind::Constraint | RepositoryErrorKind::CorruptData => {
            DeviceAuthorizationFailureKind::Internal
        }
    };
    failure(operation, kind)
}

const fn internal_failure(operation: &'static str) -> DeviceAuthorizationFailure {
    failure(operation, DeviceAuthorizationFailureKind::Internal)
}

const fn failure(
    operation: &'static str,
    kind: DeviceAuthorizationFailureKind,
) -> DeviceAuthorizationFailure {
    DeviceAuthorizationFailure::new(operation, kind)
}

#[cfg(test)]
mod tests {
    use super::{
        DeviceAuthorizationPolicy, DeviceRequestProof, DeviceRequestProofError,
        DeviceRequestProofPayload, encode_hex,
    };
    use crate::ports::{DeviceSignature, SecretDigest, SecretValue};
    use agent_room_domain::{
        ids::DeviceId,
        time::{DurationMillis, UtcMillis},
    };
    use uuid::Uuid;

    #[test]
    fn 请求证明拒绝非规范方法与可逃逸目标() {
        let invalid_method = proof("get", "/rooms");
        assert_eq!(invalid_method, Err(DeviceRequestProofError::InvalidMethod));

        let invalid_target = proof("GET", "//attacker.example");
        assert_eq!(
            invalid_target,
            Err(DeviceRequestProofError::InvalidRequestTarget)
        );
    }

    #[test]
    fn 敏感证明字段不会出现在调试输出() {
        let proof = proof("POST", "/device-sessions/refresh").expect("证明有效");
        let debug = format!("{proof:?}");

        assert!(!debug.contains("nonce-must-stay-secret"));
        assert!(!debug.contains(&"7".repeat(64)));
        assert!(debug.contains("[已脱敏]"));
    }

    #[test]
    fn 策略拒绝访问令牌活得不比刷新令牌短的配置() {
        let duration = DurationMillis::new(60_000).expect("时长有效");
        assert!(
            DeviceAuthorizationPolicy::new(
                duration,
                duration,
                duration,
                duration,
                duration,
                "matrix.example.test"
            )
            .is_err()
        );
    }

    #[test]
    fn 摘要使用稳定小写十六进制编码() {
        assert_eq!(encode_hex(&[0, 15, 16, 255]), "000f10ff");
    }

    fn proof(
        method: &str,
        request_target: &str,
    ) -> Result<DeviceRequestProof, DeviceRequestProofError> {
        let payload = DeviceRequestProofPayload::new(
            DeviceId::from_uuid(Uuid::from_u128(1)),
            UtcMillis::new(1_000).expect("时间有效"),
            SecretValue::new("nonce-must-stay-secret").expect("nonce 有效"),
            method.to_owned(),
            request_target.to_owned(),
            SecretDigest::from_array([3; 32]),
        )?;
        Ok(DeviceRequestProof::new(
            payload,
            DeviceSignature::new(vec![7; 64]).expect("签名长度有效"),
        ))
    }
}
