use std::{net::IpAddr, num::NonZeroU16, time::Duration};

use thiserror::Error;
use url::Url;

const MAX_REQUEST_TIMEOUT: Duration = Duration::from_mins(2);
const DEFAULT_SYNC_TIMELINE_LIMIT: NonZeroU16 = NonZeroU16::new(50).expect("默认值非零");
const MAX_SYNC_TIMELINE_LIMIT: u16 = 1_000;

#[derive(Debug, Clone)]
pub struct MatrixSdkConfiguration {
    homeserver_url: Url,
    request_timeout: Duration,
    sync_timeline_limit: NonZeroU16,
}

impl MatrixSdkConfiguration {
    /// 创建 Matrix SDK 网络配置。
    ///
    /// # Errors
    ///
    /// 生产地址不是 HTTPS、回环开发地址不安全或请求超时越界时返回配置错误。
    pub fn new(
        homeserver_url: impl AsRef<str>,
        request_timeout: Duration,
    ) -> Result<Self, MatrixSdkConfigurationError> {
        let homeserver_url = Url::parse(homeserver_url.as_ref())
            .map_err(|_| MatrixSdkConfigurationError::InvalidHomeserverUrl)?;
        validate_homeserver_url(&homeserver_url)?;
        if request_timeout.is_zero() || request_timeout > MAX_REQUEST_TIMEOUT {
            return Err(MatrixSdkConfigurationError::InvalidRequestTimeout);
        }
        Ok(Self {
            homeserver_url,
            request_timeout,
            sync_timeline_limit: DEFAULT_SYNC_TIMELINE_LIMIT,
        })
    }

    /// 覆盖单次同步允许返回的最大时间线事件数。
    ///
    /// # Errors
    ///
    /// 上限超过应用层单页安全边界时返回配置错误。
    pub fn with_sync_timeline_limit(
        mut self,
        limit: NonZeroU16,
    ) -> Result<Self, MatrixSdkConfigurationError> {
        if limit.get() > MAX_SYNC_TIMELINE_LIMIT {
            return Err(MatrixSdkConfigurationError::InvalidSyncTimelineLimit);
        }
        self.sync_timeline_limit = limit;
        Ok(self)
    }

    pub const fn homeserver_url(&self) -> &Url {
        &self.homeserver_url
    }

    pub const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    pub const fn sync_timeline_limit(&self) -> NonZeroU16 {
        self.sync_timeline_limit
    }
}

fn validate_homeserver_url(url: &Url) -> Result<(), MatrixSdkConfigurationError> {
    if url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(MatrixSdkConfigurationError::InvalidHomeserverUrl);
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback(url) => Ok(()),
        _ => Err(MatrixSdkConfigurationError::InsecureHomeserverUrl),
    }
}

fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MatrixSdkConfigurationError {
    #[error("Matrix Homeserver 地址无效")]
    InvalidHomeserverUrl,
    #[error("Matrix Homeserver 生产地址必须使用 HTTPS")]
    InsecureHomeserverUrl,
    #[error("Matrix 请求超时必须处于 1 毫秒到 120 秒之间")]
    InvalidRequestTimeout,
    #[error("Matrix 单次同步时间线事件上限不能超过 1000")]
    InvalidSyncTimelineLimit,
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroU16, time::Duration};

    use super::{MatrixSdkConfiguration, MatrixSdkConfigurationError};

    #[test]
    fn 只允许_https_或严格回环_http() {
        assert!(
            MatrixSdkConfiguration::new("https://matrix.example.org", Duration::from_secs(10))
                .is_ok()
        );
        assert!(
            MatrixSdkConfiguration::new("http://127.0.0.1:8008", Duration::from_secs(10)).is_ok()
        );
        assert_eq!(
            MatrixSdkConfiguration::new("http://matrix.example.org", Duration::from_secs(10))
                .expect_err("公网明文地址必须失败"),
            MatrixSdkConfigurationError::InsecureHomeserverUrl
        );
    }

    #[test]
    fn 拒绝凭据_查询和无界超时() {
        for url in [
            "https://user:secret@matrix.example.org",
            "https://matrix.example.org?token=secret",
            "https://matrix.example.org#fragment",
        ] {
            assert!(MatrixSdkConfiguration::new(url, Duration::from_secs(10)).is_err());
        }
        assert!(MatrixSdkConfiguration::new("https://matrix.example.org", Duration::ZERO).is_err());
    }

    #[test]
    fn 同步时间线默认有界且拒绝超大响应() {
        let configuration =
            MatrixSdkConfiguration::new("https://matrix.example.org", Duration::from_secs(10))
                .expect("配置有效");
        assert_eq!(configuration.sync_timeline_limit().get(), 50);
        assert!(
            configuration
                .with_sync_timeline_limit(NonZeroU16::new(1_001).expect("测试值非零"))
                .is_err()
        );
    }
}
