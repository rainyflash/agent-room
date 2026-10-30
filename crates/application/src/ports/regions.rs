//! 按来源 IP 查国家（`specs/measurement/design.md` 第 3 步）：只用来判断粗地区。

use std::net::IpAddr;

use agent_room_domain::regions::CountryCode;

/// 地址所在的国家。实现只查不记：不存地址，不写进日志、指标或错误。
pub trait CountryLookup: Send + Sync {
    /// 查不出来回 `None`，按未知地区算。
    fn country(&self, address: IpAddr) -> Option<CountryCode>;
}

/// 没有 IP 库：一律查不出来。
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCountryLookup;

impl CountryLookup for NoCountryLookup {
    fn country(&self, _address: IpAddr) -> Option<CountryCode> {
        None
    }
}
