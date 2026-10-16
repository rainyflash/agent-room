//! 网络 Agent 的来源：限流只看来源地址按 UTC 日加盐后的摘要。
//!
//! 聊天助手平台（网页版的聊天助手、MCP 目录的网关）替很多人从同一段出口地址发请求，按地址算的话
//! 整个平台的用户共用一个来源的名额。部署配置把这样的一段记成“共享出口”：整段算一个来源，按来源计的
//! 名额（进公开大厅、敲门、猜错口令）都乘上它的分量（`specs/launch-capacity/design.md` 第 13 步）。

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Mutex, PoisonError},
};

use agent_room_application::ports::NetworkSourceWeights;
use agent_room_identity_adapter::NetworkSourceDigester;
use axum::http::HeaderMap;

/// 一段共享出口：名字只用来算摘要和排查，分量至少是 1。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SharedSource {
    name: String,
    ranges: Vec<AddressRange>,
    weight: u32,
}

/// 一个网段，例如 `160.79.104.0/21`、`2607:6bc0::/48`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AddressRange {
    network: IpAddr,
    prefix: u8,
}

impl AddressRange {
    fn parse(value: &str) -> Result<Self, String> {
        let (address, prefix) = value
            .split_once('/')
            .ok_or_else(|| format!("{value} 要写成 地址/前缀长度"))?;
        let network: IpAddr = address.parse().map_err(|_| format!("{value} 的地址不对"))?;
        let maximum = if network.is_ipv4() { 32 } else { 128 };
        let prefix: u8 = prefix
            .parse()
            .ok()
            .filter(|prefix| *prefix <= maximum)
            .ok_or_else(|| format!("{value} 的前缀长度不对"))?;
        Ok(Self { network, prefix })
    }

    fn contains(&self, address: IpAddr) -> bool {
        match (self.network, address) {
            (IpAddr::V4(network), IpAddr::V4(address)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(network) & mask == u32::from(address) & mask
            }
            (IpAddr::V6(network), IpAddr::V6(address)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(network) & mask == u128::from(address) & mask
            }
            _ => false,
        }
    }
}

/// 读部署配置渲染出来的共享出口：`名字@分量:网段,网段+名字@分量:网段`。空的就是没有。
/// 只用 `_dotenv_value` 不加引号的字符，免得经过 Compose 的引号处理。
pub(crate) fn parse_shared_sources(value: &str) -> Result<Vec<SharedSource>, String> {
    let mut sources: Vec<SharedSource> = Vec::new();
    for entry in value
        .split('+')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let (label, ranges) = entry
            .split_once(':')
            .ok_or_else(|| format!("{entry} 要写成 名字@分量:网段"))?;
        let (name, weight) = label
            .split_once('@')
            .ok_or_else(|| format!("{label} 要写成 名字@分量"))?;
        if name.is_empty()
            || !name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
        {
            return Err(format!("共享出口的名字 {name} 只能用字母、数字和 -"));
        }
        if sources.iter().any(|source| source.name == name) {
            return Err(format!("共享出口 {name} 写了两次"));
        }
        let weight: u32 = weight
            .parse()
            .ok()
            .filter(|weight| (1..=1_000).contains(weight))
            .ok_or_else(|| format!("共享出口 {name} 的分量要在 1–1000 之间"))?;
        let ranges = ranges
            .split(',')
            .map(str::trim)
            .filter(|range| !range.is_empty())
            .map(AddressRange::parse)
            .collect::<Result<Vec<_>, _>>()?;
        if ranges.is_empty() {
            return Err(format!("共享出口 {name} 没有网段"));
        }
        sources.push(SharedSource {
            name: name.to_owned(),
            ranges,
            weight,
        });
    }
    Ok(sources)
}

/// 算来源摘要，并记住哪些摘要是共享出口的：摘要按天加盐，应用层认不出来，只能问这里。
pub(crate) struct NetworkSources {
    digester: NetworkSourceDigester,
    shared: Vec<SharedSource>,
    /// 共享出口的摘要 → （UTC 日，分量）。只留当天和前一天的。
    weights: Mutex<HashMap<[u8; 32], (i64, u32)>>,
}

impl NetworkSources {
    pub(crate) fn new(digester: NetworkSourceDigester, shared: Vec<SharedSource>) -> Self {
        Self {
            digester,
            shared,
            weights: Mutex::new(HashMap::new()),
        }
    }

    /// 来源地址按 UTC 日加盐后的摘要。共享出口里的地址都算成同一个来源。
    pub(crate) fn digest(&self, headers: &HeaderMap, utc_day: i64) -> [u8; 32] {
        let address = forwarded_address(headers);
        let shared = address.and_then(|address| {
            self.shared
                .iter()
                .find(|source| source.ranges.iter().any(|range| range.contains(address)))
        });
        let Some(shared) = shared else {
            return self
                .digester
                .digest(&address_source(address), &utc_day.to_string());
        };
        let digest = self
            .digester
            .digest(&format!("shared:{}", shared.name), &utc_day.to_string());
        // 没配封存密钥时所有摘要都是全零，别把它记成共享出口。
        if digest != [0; 32] {
            let mut weights = self.weights.lock().unwrap_or_else(PoisonError::into_inner);
            weights.retain(|_, (day, _)| *day >= utc_day - 1);
            weights.insert(digest, (utc_day, shared.weight));
        }
        digest
    }
}

impl NetworkSourceWeights for NetworkSources {
    fn weight(&self, source: &[u8; 32]) -> u32 {
        self.weights
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(source)
            .map_or(1, |(_, weight)| *weight)
    }
}

/// Caddy 不信任上游，自己把真实地址写进 `X-Forwarded-For`；取最后一个值，也就是离控制面最近的
/// 那一跳写入的。
fn forwarded_address(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .rfind(|value| !value.is_empty())
        .and_then(|value| value.parse::<IpAddr>().ok())
}

/// IPv6 按 /64 归并：同一网段里换地址绕不过限流。取不到地址时所有这类请求共用一个来源（只在本地
/// 开发时出现）。
fn address_source(address: Option<IpAddr>) -> String {
    match address {
        Some(IpAddr::V4(address)) => address.to_string(),
        Some(IpAddr::V6(address)) => address.to_ipv4_mapped().map_or_else(
            || {
                let [a, b, c, d, ..] = address.segments();
                format!("{a:x}:{b:x}:{c:x}:{d:x}::/64")
            },
            |mapped| mapped.to_string(),
        ),
        None => "unknown".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use agent_room_identity_adapter::NetworkAgentSealKey;
    use axum::http::HeaderValue;

    use super::*;

    fn headers(address: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_str(address).expect("地址能当请求头"),
        );
        headers
    }

    fn sources(shared: &str) -> NetworkSources {
        let key = NetworkAgentSealKey::from_bytes([7; 32]);
        NetworkSources::new(
            NetworkSourceDigester::new(Some(&key)),
            parse_shared_sources(shared).expect("配置能读"),
        )
    }

    #[test]
    fn 共享出口整段算一个来源_分量只给这一段() {
        let sources =
            sources("assistant@20:160.79.104.0/21,2607:6bc0::/48+gateway@5:203.0.113.7/32");
        let first = sources.digest(&headers("160.79.104.9"), 100);
        let second = sources.digest(&headers("160.79.111.250"), 100);
        let ipv6 = sources.digest(&headers("2607:6bc0:0:1::5"), 100);
        assert_eq!(first, second, "同一段里的地址是同一个来源");
        assert_eq!(first, ipv6, "同一个共享出口的 IPv4 和 IPv6 也是同一个来源");
        assert_eq!(sources.weight(&first), 20);

        let gateway = sources.digest(&headers("203.0.113.7"), 100);
        assert_ne!(gateway, first);
        assert_eq!(sources.weight(&gateway), 5);

        let outside = sources.digest(&headers("160.79.112.1"), 100);
        assert_ne!(outside, first, "/21 外面的地址是自己的来源");
        assert_eq!(sources.weight(&outside), 1);
        assert_ne!(
            sources.digest(&headers("160.79.112.2"), 100),
            outside,
            "普通地址照旧各算各的"
        );
    }

    #[test]
    fn 换天以后摘要换了_前一天的还认得_再早的忘掉() {
        let sources = sources("assistant@20:160.79.104.0/21");
        let monday = sources.digest(&headers("160.79.104.9"), 100);
        let tuesday = sources.digest(&headers("160.79.104.9"), 101);
        assert_ne!(monday, tuesday);
        assert_eq!(sources.weight(&monday), 20, "跨零点的那一下还认得前一天的");
        assert_eq!(sources.weight(&tuesday), 20);
        let _ = sources.digest(&headers("160.79.104.9"), 102);
        assert_eq!(sources.weight(&monday), 1, "再早的不留");
    }

    #[test]
    fn 没配共享出口时和以前一样() {
        let sources = sources("");
        let digest = sources.digest(&headers("160.79.104.9"), 100);
        assert_eq!(sources.weight(&digest), 1);
        assert_eq!(
            sources.digest(&headers("2001:db8:1:2:3::1"), 100),
            sources.digest(&headers("2001:db8:1:2:ffff::9"), 100),
            "IPv6 照旧按 /64 归并"
        );
    }

    #[test]
    fn 配置写错时说清楚哪里错() {
        for (value, expected) in [
            ("assistant:160.79.104.0/21", "名字@分量"),
            ("assistant@0:160.79.104.0/21", "1–1000"),
            ("assistant@20:", "没有网段"),
            ("assistant@20:160.79.104.0", "地址/前缀长度"),
            ("assistant@20:160.79.104.0/33", "前缀长度不对"),
            ("assistant@20:not-an-address/8", "地址不对"),
            ("a b@20:160.79.104.0/21", "字母、数字和 -"),
            ("x@2:10.0.0.0/8+x@3:11.0.0.0/8", "写了两次"),
        ] {
            let error = parse_shared_sources(value).expect_err(value);
            assert!(error.contains(expected), "{value}: {error}");
        }
    }
}
