use crate::{
    DomainError, DomainResult,
    ids::{DeviceId, DeviceTokenFamilyId, PrincipalId},
    time::UtcMillis,
};

const MAX_DEVICE_LABEL_LENGTH: usize = 128;
const ED25519_PUBLIC_KEY_LENGTH: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicePlatform {
    Windows,
    MacOs,
    Linux,
    Web,
}

impl DevicePlatform {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOs => "macos",
            Self::Linux => "linux",
            Self::Web => "web",
        }
    }
}

impl TryFrom<&str> for DevicePlatform {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "windows" => Ok(Self::Windows),
            "macos" => Ok(Self::MacOs),
            "linux" => Ok(Self::Linux),
            "web" => Ok(Self::Web),
            _ => Err(DomainError::Validation {
                field: "device_platform",
                reason: "包含未知平台",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceTrustState {
    Pending,
    Verified,
    Revoked,
}

impl DeviceTrustState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Verified => "verified",
            Self::Revoked => "revoked",
        }
    }
}

impl TryFrom<&str> for DeviceTrustState {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "verified" => Ok(Self::Verified),
            "revoked" => Ok(Self::Revoked),
            _ => Err(DomainError::Validation {
                field: "device_trust_state",
                reason: "包含未知状态",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevicePublicSigningKey([u8; ED25519_PUBLIC_KEY_LENGTH]);

impl DevicePublicSigningKey {
    /// 从 Ed25519 公钥字节创建设备签名键。
    ///
    /// # Errors
    ///
    /// 字节长度不是 32 时返回校验错误。
    pub fn new(bytes: Vec<u8>) -> DomainResult<Self> {
        let bytes = <[u8; ED25519_PUBLIC_KEY_LENGTH]>::try_from(bytes).map_err(|_| {
            DomainError::Validation {
                field: "device_public_signing_key",
                reason: "必须是 32 字节 Ed25519 公钥",
            }
        })?;
        Ok(Self(bytes))
    }

    pub const fn as_bytes(&self) -> &[u8; ED25519_PUBLIC_KEY_LENGTH] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    id: DeviceId,
    principal_id: PrincipalId,
    label: String,
    platform: DevicePlatform,
    public_signing_key: DevicePublicSigningKey,
    matrix_device_id: Option<String>,
    trust_state: DeviceTrustState,
    last_seen_at: Option<UtcMillis>,
    revoked_at: Option<UtcMillis>,
    created_at: UtcMillis,
}

impl Device {
    /// 创建尚未完成持有证明的设备。
    ///
    /// # Errors
    ///
    /// 标签为空、超长或包含控制字符时返回校验错误。
    pub fn register(
        id: DeviceId,
        principal_id: PrincipalId,
        label: String,
        platform: DevicePlatform,
        public_signing_key: DevicePublicSigningKey,
        created_at: UtcMillis,
    ) -> DomainResult<Self> {
        validate_label(&label)?;
        Ok(Self {
            id,
            principal_id,
            label,
            platform,
            public_signing_key,
            matrix_device_id: None,
            trust_state: DeviceTrustState::Pending,
            last_seen_at: None,
            revoked_at: None,
            created_at,
        })
    }

    /// 从持久化状态恢复设备并重新检查跨字段约束。
    ///
    /// # Errors
    ///
    /// 字段非法或撤销时间与信任状态不一致时返回校验错误。
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: DeviceId,
        principal_id: PrincipalId,
        label: String,
        platform: DevicePlatform,
        public_signing_key: DevicePublicSigningKey,
        matrix_device_id: Option<String>,
        trust_state: DeviceTrustState,
        last_seen_at: Option<UtcMillis>,
        revoked_at: Option<UtcMillis>,
        created_at: UtcMillis,
    ) -> DomainResult<Self> {
        validate_label(&label)?;
        if matrix_device_id.as_deref().is_some_and(|value| {
            value.is_empty() || value.len() > 255 || value.chars().any(char::is_control)
        }) {
            return Err(DomainError::Validation {
                field: "matrix_device_id",
                reason: "长度超限或包含控制字符",
            });
        }
        if matches!(trust_state, DeviceTrustState::Revoked) != revoked_at.is_some() {
            return Err(DomainError::Validation {
                field: "device_revoked_at",
                reason: "必须与撤销状态一致",
            });
        }
        if revoked_at.is_some_and(|value| value < created_at)
            || last_seen_at.is_some_and(|value| value < created_at)
        {
            return Err(DomainError::Validation {
                field: "device_timestamp",
                reason: "不能早于设备创建时间",
            });
        }
        Ok(Self {
            id,
            principal_id,
            label,
            platform,
            public_signing_key,
            matrix_device_id,
            trust_state,
            last_seen_at,
            revoked_at,
            created_at,
        })
    }

    pub const fn id(&self) -> DeviceId {
        self.id
    }

    pub const fn principal_id(&self) -> PrincipalId {
        self.principal_id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn platform(&self) -> DevicePlatform {
        self.platform
    }

    pub const fn public_signing_key(&self) -> &DevicePublicSigningKey {
        &self.public_signing_key
    }

    pub fn matrix_device_id(&self) -> Option<&str> {
        self.matrix_device_id.as_deref()
    }

    pub const fn trust_state(&self) -> DeviceTrustState {
        self.trust_state
    }

    pub const fn last_seen_at(&self) -> Option<UtcMillis> {
        self.last_seen_at
    }

    pub const fn revoked_at(&self) -> Option<UtcMillis> {
        self.revoked_at
    }

    pub const fn created_at(&self) -> UtcMillis {
        self.created_at
    }

    pub const fn accepts_authenticated_requests(&self) -> bool {
        matches!(self.trust_state, DeviceTrustState::Verified)
    }

    /// 完成设备持有证明，重复验证保持幂等。
    ///
    /// # Errors
    ///
    /// 已撤销设备不能重新验证。
    pub fn verify(&mut self) -> DomainResult<()> {
        match self.trust_state {
            DeviceTrustState::Pending | DeviceTrustState::Verified => {
                self.trust_state = DeviceTrustState::Verified;
                Ok(())
            }
            DeviceTrustState::Revoked => Err(DomainError::InvalidTransition {
                entity: "device",
                from: DeviceTrustState::Revoked.as_str(),
                to: DeviceTrustState::Verified.as_str(),
            }),
        }
    }

    /// 绑定独立的 Matrix Device 标识。
    ///
    /// # Errors
    ///
    /// 设备未验证、已撤销或标识非法时返回错误。
    pub fn bind_matrix_device(&mut self, matrix_device_id: String) -> DomainResult<()> {
        if !self.accepts_authenticated_requests() {
            return Err(DomainError::InvalidTransition {
                entity: "device",
                from: self.trust_state.as_str(),
                to: "matrix_bound",
            });
        }
        if matrix_device_id.is_empty()
            || matrix_device_id.len() > 255
            || matrix_device_id.chars().any(char::is_control)
        {
            return Err(DomainError::Validation {
                field: "matrix_device_id",
                reason: "长度超限或包含控制字符",
            });
        }
        match self.matrix_device_id.as_deref() {
            None => self.matrix_device_id = Some(matrix_device_id),
            Some(existing) if existing == matrix_device_id => {}
            Some(_) => {
                return Err(DomainError::InvalidTransition {
                    entity: "device",
                    from: "matrix_bound",
                    to: "matrix_rebound",
                });
            }
        }
        Ok(())
    }

    /// 记录通过认证的设备活动时间。
    ///
    /// # Errors
    ///
    /// 设备未验证、已撤销或时间倒退时返回错误。
    pub fn record_seen(&mut self, seen_at: UtcMillis) -> DomainResult<()> {
        if !self.accepts_authenticated_requests() {
            return Err(DomainError::InvalidTransition {
                entity: "device",
                from: self.trust_state.as_str(),
                to: "seen",
            });
        }
        if seen_at < self.created_at
            || self
                .last_seen_at
                .is_some_and(|last_seen_at| seen_at < last_seen_at)
        {
            return Err(DomainError::Validation {
                field: "device_last_seen_at",
                reason: "不能倒退",
            });
        }
        self.last_seen_at = Some(seen_at);
        Ok(())
    }

    /// 撤销设备，重复撤销保留首次时间。
    ///
    /// # Errors
    ///
    /// 撤销时间早于创建时间时返回校验错误。
    pub fn revoke(&mut self, revoked_at: UtcMillis) -> DomainResult<()> {
        if revoked_at < self.created_at {
            return Err(DomainError::Validation {
                field: "device_revoked_at",
                reason: "不能早于设备创建时间",
            });
        }
        if !matches!(self.trust_state, DeviceTrustState::Revoked) {
            self.trust_state = DeviceTrustState::Revoked;
            self.revoked_at = Some(revoked_at);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceTokenFamilyState {
    Active,
    Revoked,
    Compromised,
}

impl DeviceTokenFamilyState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
            Self::Compromised => "compromised",
        }
    }
}

impl TryFrom<&str> for DeviceTokenFamilyState {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "active" => Ok(Self::Active),
            "revoked" => Ok(Self::Revoked),
            "compromised" => Ok(Self::Compromised),
            _ => Err(DomainError::Validation {
                field: "device_token_family_state",
                reason: "包含未知状态",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceTokenFamily {
    id: DeviceTokenFamilyId,
    device_id: DeviceId,
    state: DeviceTokenFamilyState,
    created_at: UtcMillis,
    expires_at: UtcMillis,
    revoked_at: Option<UtcMillis>,
    compromise_detected_at: Option<UtcMillis>,
}

impl DeviceTokenFamily {
    /// 创建设备范围 Token 族。
    ///
    /// # Errors
    ///
    /// 到期时间没有严格晚于创建时间时返回校验错误。
    pub fn new(
        id: DeviceTokenFamilyId,
        device_id: DeviceId,
        created_at: UtcMillis,
        expires_at: UtcMillis,
    ) -> DomainResult<Self> {
        if expires_at <= created_at {
            return Err(DomainError::Validation {
                field: "device_token_family_expires_at",
                reason: "必须晚于创建时间",
            });
        }
        Ok(Self {
            id,
            device_id,
            state: DeviceTokenFamilyState::Active,
            created_at,
            expires_at,
            revoked_at: None,
            compromise_detected_at: None,
        })
    }

    /// 从持久化状态恢复 Token 族并检查状态时间一致性。
    ///
    /// # Errors
    ///
    /// 状态、过期时间、撤销时间或泄露时间不一致时返回校验错误。
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        id: DeviceTokenFamilyId,
        device_id: DeviceId,
        state: DeviceTokenFamilyState,
        created_at: UtcMillis,
        expires_at: UtcMillis,
        revoked_at: Option<UtcMillis>,
        compromise_detected_at: Option<UtcMillis>,
    ) -> DomainResult<Self> {
        if expires_at <= created_at
            || revoked_at.is_some_and(|value| value < created_at)
            || compromise_detected_at.is_some_and(|value| value < created_at)
            || matches!(state, DeviceTokenFamilyState::Active)
                && (revoked_at.is_some() || compromise_detected_at.is_some())
            || matches!(state, DeviceTokenFamilyState::Revoked)
                && (revoked_at.is_none() || compromise_detected_at.is_some())
            || matches!(state, DeviceTokenFamilyState::Compromised)
                && (revoked_at.is_none() || compromise_detected_at.is_none())
        {
            return Err(DomainError::Validation {
                field: "device_token_family_state",
                reason: "状态与时间字段不一致",
            });
        }
        Ok(Self {
            id,
            device_id,
            state,
            created_at,
            expires_at,
            revoked_at,
            compromise_detected_at,
        })
    }

    pub const fn id(&self) -> DeviceTokenFamilyId {
        self.id
    }

    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    pub const fn state(&self) -> DeviceTokenFamilyState {
        self.state
    }

    pub const fn created_at(&self) -> UtcMillis {
        self.created_at
    }

    pub const fn expires_at(&self) -> UtcMillis {
        self.expires_at
    }

    pub const fn revoked_at(&self) -> Option<UtcMillis> {
        self.revoked_at
    }

    pub const fn compromise_detected_at(&self) -> Option<UtcMillis> {
        self.compromise_detected_at
    }

    pub fn allows_rotation(&self, now: UtcMillis) -> bool {
        matches!(self.state, DeviceTokenFamilyState::Active) && now < self.expires_at
    }

    /// 正常撤销 Token 族，重复撤销保持首次时间。
    ///
    /// # Errors
    ///
    /// 时间早于创建时间时返回错误；已标记泄露的 Token 族不会降级为普通撤销。
    pub fn revoke(&mut self, revoked_at: UtcMillis) -> DomainResult<()> {
        if revoked_at < self.created_at {
            return Err(DomainError::Validation {
                field: "device_token_family_revoked_at",
                reason: "不能早于创建时间",
            });
        }
        if matches!(self.state, DeviceTokenFamilyState::Active) {
            self.state = DeviceTokenFamilyState::Revoked;
            self.revoked_at = Some(revoked_at);
        }
        Ok(())
    }

    /// 将刷新令牌重用升级为整个 Token 族泄露。
    ///
    /// # Errors
    ///
    /// 检测时间早于创建时间时返回错误。
    pub fn mark_compromised(&mut self, detected_at: UtcMillis) -> DomainResult<()> {
        if detected_at < self.created_at {
            return Err(DomainError::Validation {
                field: "device_token_family_compromise_detected_at",
                reason: "不能早于创建时间",
            });
        }
        if !matches!(self.state, DeviceTokenFamilyState::Compromised) {
            self.state = DeviceTokenFamilyState::Compromised;
            self.revoked_at.get_or_insert(detected_at);
            self.compromise_detected_at = Some(detected_at);
        }
        Ok(())
    }
}

fn validate_label(label: &str) -> DomainResult<()> {
    if label.is_empty()
        || label.len() > MAX_DEVICE_LABEL_LENGTH
        || label.chars().any(char::is_control)
    {
        return Err(DomainError::Validation {
            field: "device_label",
            reason: "不能为空、超长或包含控制字符",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        Device, DevicePlatform, DevicePublicSigningKey, DeviceTokenFamily, DeviceTokenFamilyState,
        DeviceTrustState,
    };
    use crate::{
        ids::{DeviceId, DeviceTokenFamilyId, PrincipalId},
        time::UtcMillis,
    };

    #[test]
    fn 已撤销设备不能复活且撤销保持首次时间() {
        let mut device = device();
        device.verify().expect("设备持有证明有效");
        device.revoke(time(2_000)).expect("首次撤销成功");
        device.revoke(time(3_000)).expect("重复撤销幂等");

        assert_eq!(device.trust_state(), DeviceTrustState::Revoked);
        assert_eq!(device.revoked_at(), Some(time(2_000)));
        assert!(!device.accepts_authenticated_requests());
        assert!(device.verify().is_err());
    }

    #[test]
    fn 只有已验证设备可以绑定_matrix_设备和记录活动() {
        let mut device = device();
        assert!(device.record_seen(time(2_000)).is_err());
        assert!(device.bind_matrix_device("MATRIX-1".to_owned()).is_err());

        device.verify().expect("设备验证成功");
        device
            .bind_matrix_device("MATRIX-1".to_owned())
            .expect("首次绑定成功");
        device
            .bind_matrix_device("MATRIX-1".to_owned())
            .expect("重复绑定幂等");
        assert!(device.bind_matrix_device("MATRIX-2".to_owned()).is_err());

        device.record_seen(time(3_000)).expect("活动时间有效");
        assert!(device.record_seen(time(2_000)).is_err());
    }

    #[test]
    fn 刷新令牌重用会把整个族标记为泄露且不可再轮换() {
        let mut family = DeviceTokenFamily::new(
            DeviceTokenFamilyId::from_uuid(Uuid::from_u128(3)),
            DeviceId::from_uuid(Uuid::from_u128(2)),
            time(1_000),
            time(10_000),
        )
        .expect("Token 族有效");
        assert!(family.allows_rotation(time(2_000)));

        family.mark_compromised(time(3_000)).expect("泄露状态有效");
        family
            .mark_compromised(time(4_000))
            .expect("重复检测保持幂等");

        assert_eq!(family.state(), DeviceTokenFamilyState::Compromised);
        assert_eq!(family.revoked_at(), Some(time(3_000)));
        assert_eq!(family.compromise_detected_at(), Some(time(3_000)));
        assert!(!family.allows_rotation(time(4_000)));
    }

    #[test]
    fn 设备公钥严格要求_ed25519_长度() {
        assert!(DevicePublicSigningKey::new(vec![7; 31]).is_err());
        assert!(DevicePublicSigningKey::new(vec![7; 32]).is_ok());
        assert!(DevicePublicSigningKey::new(vec![7; 33]).is_err());
    }

    fn device() -> Device {
        Device::register(
            DeviceId::from_uuid(Uuid::from_u128(2)),
            PrincipalId::from_uuid(Uuid::from_u128(1)),
            "开发工作站".to_owned(),
            DevicePlatform::Windows,
            DevicePublicSigningKey::new(vec![7; 32]).expect("测试公钥有效"),
            time(1_000),
        )
        .expect("测试设备有效")
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
