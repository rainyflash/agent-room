const SECURE_STORAGE_SERVICE_VARIABLE: &str = "AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE";
const MIN_SERVICE_LENGTH: usize = 3;
const MAX_SERVICE_LENGTH: usize = 128;

pub const DEFAULT_SECURE_STORAGE_SERVICE: &str = "dev.agent-room.bridge";

/// Bridge 相关进程共享的操作系统安全存储服务名。
///
/// 该值是本地安全边界的一部分：Bridge、MCP 与桌面壳必须使用同一个值，
/// 同时测试或并行安装可以使用互不污染的命名空间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecureStorageService(String);

impl SecureStorageService {
    /// 校验并创建安全存储服务名。
    ///
    /// # Errors
    ///
    /// 名称长度越界、首尾不是字母数字或包含不受支持字符时返回错误。
    pub fn new(value: impl Into<String>) -> Result<Self, SecureStorageServiceFailure> {
        let value = value.into();
        let trimmed = value.trim();
        let valid_length = (MIN_SERVICE_LENGTH..=MAX_SERVICE_LENGTH).contains(&trimmed.len());
        let valid_edges = trimmed
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            && trimmed
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric);
        let valid_characters = trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
        if !valid_length || !valid_edges || !valid_characters {
            return Err(SecureStorageServiceFailure);
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl Default for SecureStorageService {
    fn default() -> Self {
        Self(DEFAULT_SECURE_STORAGE_SERVICE.to_owned())
    }
}

/// 从当前进程环境读取安全存储命名空间。
///
/// # Errors
///
/// 显式值不符合安全命名规则时返回错误；未配置时使用生产默认值。
pub fn secure_storage_service_from_environment()
-> Result<SecureStorageService, SecureStorageServiceFailure> {
    resolve_secure_storage_service(|name| std::env::var(name).ok())
}

/// 让 Bridge、MCP 和桌面壳通过同一规则解析安全存储命名空间。
///
/// # Errors
///
/// 显式值不符合安全命名规则时返回错误。
pub fn resolve_secure_storage_service(
    mut read: impl FnMut(&'static str) -> Option<String>,
) -> Result<SecureStorageService, SecureStorageServiceFailure> {
    match read(SECURE_STORAGE_SERVICE_VARIABLE).filter(|value| !value.trim().is_empty()) {
        Some(value) => SecureStorageService::new(value),
        None => Ok(SecureStorageService::default()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecureStorageServiceFailure;

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SECURE_STORAGE_SERVICE, SecureStorageService, resolve_secure_storage_service,
    };

    #[test]
    fn 未配置时保持生产安全存储命名空间不变() {
        let service = resolve_secure_storage_service(|_| None).expect("默认服务名有效");

        assert_eq!(service.as_str(), DEFAULT_SECURE_STORAGE_SERVICE);
    }

    #[test]
    fn 纵向测试可使用隔离命名空间() {
        let service = resolve_secure_storage_service(|name| {
            (name == "AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE")
                .then(|| "dev.agent-room.bridge.vertical-24".to_owned())
        })
        .expect("隔离服务名有效");

        assert_eq!(service.as_str(), "dev.agent-room.bridge.vertical-24");
    }

    #[test]
    fn 拒绝可能破坏密钥环寻址的服务名() {
        for invalid in [
            "ab",
            ".agent-room",
            "agent-room.",
            "agent room",
            "测试命名空间",
        ] {
            assert!(
                SecureStorageService::new(invalid).is_err(),
                "应拒绝 {invalid}"
            );
        }
    }
}
