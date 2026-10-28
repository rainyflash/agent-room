//! 请求的真实来源地址。网络 Agent 的限流和按粗地区汇总都用它，用完就丢。

use std::net::IpAddr;

use axum::http::HeaderMap;

/// Caddy 不信任上游，自己把真实地址写进 `X-Forwarded-For`；取最后一个值，也就是离控制面最近的
/// 那一跳写入的。
pub(crate) fn forwarded_address(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .rfind(|value| !value.is_empty())
        .and_then(|value| value.parse::<IpAddr>().ok())
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::forwarded_address;

    #[test]
    fn 取离控制面最近的那一跳写的地址() {
        let mut headers = HeaderMap::new();
        assert_eq!(forwarded_address(&headers), None);
        headers.append(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.9, 198.51.100.7"),
        );
        assert_eq!(
            forwarded_address(&headers),
            Some("198.51.100.7".parse().unwrap())
        );
        headers.append("x-forwarded-for", HeaderValue::from_static("2001:db8::1"));
        assert_eq!(
            forwarded_address(&headers),
            Some("2001:db8::1".parse().unwrap())
        );
        headers.append(
            "x-forwarded-for",
            HeaderValue::from_static("not-an-address"),
        );
        assert_eq!(forwarded_address(&headers), None);
    }
}
