use std::{fmt, net::IpAddr, time::Duration};

use agent_room_application::ports::SecretValue;
use thiserror::Error;
use url::Url;

const MAX_BUCKET_LENGTH: usize = 63;
const MAX_REGION_LENGTH: usize = 64;

#[derive(Clone)]
pub struct S3ContentStoreConfig {
    endpoint: Url,
    bucket: String,
    region: String,
    access_key_id: SecretValue,
    secret_access_key: SecretValue,
    operation_timeout: Duration,
}

impl S3ContentStoreConfig {
    /// 创建私有 S3 兼容对象存储配置。
    ///
    /// # Errors
    ///
    /// 端点不安全、桶名或区域非法、超时为零时返回错误。
    pub fn new(
        endpoint: impl AsRef<str>,
        bucket: impl Into<String>,
        region: impl Into<String>,
        access_key_id: SecretValue,
        secret_access_key: SecretValue,
        operation_timeout: Duration,
    ) -> Result<Self, S3ContentStoreConfigError> {
        let endpoint = Url::parse(endpoint.as_ref())
            .map_err(|_| S3ContentStoreConfigError::InvalidEndpoint)?;
        validate_endpoint(&endpoint)?;

        let bucket = bucket.into();
        validate_bucket(&bucket)?;

        let region = region.into();
        validate_region(&region)?;

        if operation_timeout.is_zero() {
            return Err(S3ContentStoreConfigError::InvalidTimeout);
        }

        Ok(Self {
            endpoint,
            bucket,
            region,
            access_key_id,
            secret_access_key,
            operation_timeout,
        })
    }

    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub fn region(&self) -> &str {
        &self.region
    }

    pub fn access_key_id(&self) -> &SecretValue {
        &self.access_key_id
    }

    pub fn secret_access_key(&self) -> &SecretValue {
        &self.secret_access_key
    }

    pub const fn operation_timeout(&self) -> Duration {
        self.operation_timeout
    }
}

impl fmt::Debug for S3ContentStoreConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("S3ContentStoreConfig")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("access_key_id", &"[已脱敏]")
            .field("secret_access_key", &"[已脱敏]")
            .field("operation_timeout", &self.operation_timeout)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum S3ContentStoreConfigError {
    #[error("对象存储端点非法")]
    InvalidEndpoint,
    #[error("非回环对象存储端点必须使用 HTTPS")]
    InsecureEndpoint,
    #[error("对象存储桶名非法")]
    InvalidBucket,
    #[error("对象存储区域非法")]
    InvalidRegion,
    #[error("对象存储操作超时必须大于零")]
    InvalidTimeout,
}

fn validate_endpoint(endpoint: &Url) -> Result<(), S3ContentStoreConfigError> {
    if endpoint.cannot_be_a_base()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(S3ContentStoreConfigError::InvalidEndpoint);
    }

    match endpoint.scheme() {
        "https" => Ok(()),
        "http" if is_loopback(endpoint) => Ok(()),
        "http" => Err(S3ContentStoreConfigError::InsecureEndpoint),
        _ => Err(S3ContentStoreConfigError::InvalidEndpoint),
    }
}

fn validate_bucket(bucket: &str) -> Result<(), S3ContentStoreConfigError> {
    let valid = (3..=MAX_BUCKET_LENGTH).contains(&bucket.len())
        && bucket.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
        && bucket
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && bucket
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && !bucket.contains("..")
        && !bucket.contains(".-")
        && !bucket.contains("-.");
    if !valid {
        return Err(S3ContentStoreConfigError::InvalidBucket);
    }
    Ok(())
}

fn validate_region(region: &str) -> Result<(), S3ContentStoreConfigError> {
    let valid = !region.is_empty()
        && region.len() <= MAX_REGION_LENGTH
        && region
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if !valid {
        return Err(S3ContentStoreConfigError::InvalidRegion);
    }
    Ok(())
}

fn is_loopback(endpoint: &Url) -> bool {
    let Some(host) = endpoint.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use agent_room_application::ports::SecretValue;

    use super::{S3ContentStoreConfig, S3ContentStoreConfigError};

    #[test]
    fn 仅允许_https_或本机开发端点() {
        assert!(configuration("https://objects.example.com").is_ok());
        assert!(configuration("http://127.0.0.1:8333").is_ok());
        assert_eq!(
            configuration("http://objects.example.com").expect_err("公网明文端点必须失败"),
            S3ContentStoreConfigError::InsecureEndpoint
        );
    }

    #[test]
    fn 配置调试输出不会泄漏凭据() {
        let configuration = configuration("https://objects.example.com").expect("配置有效");
        let rendered = format!("{configuration:?}");
        assert!(!rendered.contains("test-access-key"));
        assert!(!rendered.contains("test-secret-key"));
        assert!(rendered.contains("已脱敏"));
    }

    fn configuration(endpoint: &str) -> Result<S3ContentStoreConfig, S3ContentStoreConfigError> {
        S3ContentStoreConfig::new(
            endpoint,
            "agent-room-content",
            "us-east-1",
            SecretValue::new("test-access-key").expect("访问密钥有效"),
            SecretValue::new("test-secret-key").expect("秘密密钥有效"),
            Duration::from_secs(30),
        )
    }
}
