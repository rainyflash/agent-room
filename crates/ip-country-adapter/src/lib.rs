//! 用 `MaxMind DB` 格式的 IP 库（DB-IP 的免费国家库，CC BY 4.0）查地址所在的国家
//! （`specs/measurement/design.md` 第 3 步）。只读库里的国家代码；地址查完就丢，不记日志。

use std::{io, net::IpAddr, path::Path};

use agent_room_application::ports::CountryLookup;
use agent_room_domain::regions::CountryCode;
use maxminddb::{PathElement, Reader};

const COUNTRY_CODE_PATH: [PathElement<'static>; 2] =
    [PathElement::Key("country"), PathElement::Key("iso_code")];

#[derive(Debug, thiserror::Error)]
pub enum CountryDatabaseError {
    /// 没有这个文件：控制面照常运行，地区一律算未知。
    #[error("IP 库不存在")]
    Missing,
    #[error("IP 库读不出来：{0:?}")]
    Unreadable(io::ErrorKind),
    #[error("IP 库格式不对：{0}")]
    Invalid(maxminddb::MaxMindDbError),
}

pub struct MaxMindCountryLookup {
    reader: Reader<Vec<u8>>,
}

impl MaxMindCountryLookup {
    /// 整个文件读进内存（国家库约 8 MB），之后查询不再碰磁盘。
    ///
    /// # Errors
    ///
    /// 文件不存在、读不出来或不是 `MaxMind DB` 格式时返回错误。
    pub fn open(path: &Path) -> Result<Self, CountryDatabaseError> {
        let bytes = std::fs::read(path).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => CountryDatabaseError::Missing,
            kind => CountryDatabaseError::Unreadable(kind),
        })?;
        Self::from_bytes(bytes)
    }

    /// # Errors
    ///
    /// 不是 `MaxMind DB` 格式时返回错误。
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, CountryDatabaseError> {
        Reader::from_source(bytes)
            .map(|reader| Self { reader })
            .map_err(CountryDatabaseError::Invalid)
    }

    /// 库是哪天生成的（Unix 秒），启动时记一笔，换库时好核对。
    #[must_use]
    pub fn build_epoch(&self) -> u64 {
        self.reader.metadata().build_epoch
    }
}

impl CountryLookup for MaxMindCountryLookup {
    fn country(&self, address: IpAddr) -> Option<CountryCode> {
        let found = self.reader.lookup(address).ok()?;
        let code: &str = found.decode_path(&COUNTRY_CODE_PATH).ok().flatten()?;
        CountryCode::parse(code)
    }
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::CountryLookup as _;

    use super::{CountryDatabaseError, MaxMindCountryLookup};

    /// 手写一个只有一个节点的 IPv4 库：`0.0.0.0/1` 在德国，`128.0.0.0/1` 不在库里。
    fn tiny_database() -> Vec<u8> {
        let mut bytes = Vec::new();
        // 节点 0：左边指向数据区开头（节点数 + 16 + 偏移 0），右边等于节点数，表示没有数据。
        bytes.extend_from_slice(&[0x00, 0x00, 0x11, 0x00, 0x00, 0x01]);
        bytes.extend_from_slice(&[0; 16]);
        // {"country": {"iso_code": "DE"}}
        bytes.push(0xE1);
        push_string(&mut bytes, "country");
        bytes.push(0xE1);
        push_string(&mut bytes, "iso_code");
        push_string(&mut bytes, "DE");
        bytes.extend_from_slice(b"\xab\xcd\xefMaxMind.com");
        bytes.push(0xE9);
        push_string(&mut bytes, "binary_format_major_version");
        bytes.extend_from_slice(&[0xA1, 0x02]);
        push_string(&mut bytes, "binary_format_minor_version");
        bytes.push(0xA0);
        push_string(&mut bytes, "build_epoch");
        bytes.extend_from_slice(&[0x01, 0x02, 0x2A]);
        push_string(&mut bytes, "database_type");
        push_string(&mut bytes, "Test-Country");
        push_string(&mut bytes, "description");
        bytes.push(0xE0);
        push_string(&mut bytes, "ip_version");
        bytes.extend_from_slice(&[0xA1, 0x04]);
        push_string(&mut bytes, "languages");
        bytes.extend_from_slice(&[0x00, 0x04]);
        push_string(&mut bytes, "node_count");
        bytes.extend_from_slice(&[0xC1, 0x01]);
        push_string(&mut bytes, "record_size");
        bytes.extend_from_slice(&[0xA1, 0x18]);
        bytes
    }

    fn push_string(bytes: &mut Vec<u8>, value: &str) {
        let length = u8::try_from(value.len()).expect("测试里的字符串都很短");
        assert!(length < 29);
        bytes.push(0x40 | length);
        bytes.extend_from_slice(value.as_bytes());
    }

    #[test]
    fn 查得到的给国家_查不到的和不同版本的地址给不出() {
        let lookup = MaxMindCountryLookup::from_bytes(tiny_database()).expect("手写的库能打开");
        assert_eq!(lookup.build_epoch(), 42);
        assert_eq!(
            lookup
                .country("1.2.3.4".parse().unwrap())
                .map(|code| code.as_str().to_owned()),
            Some("DE".to_owned())
        );
        assert_eq!(lookup.country("200.1.1.1".parse().unwrap()), None);
        assert_eq!(lookup.country("2001:db8::1".parse().unwrap()), None);
    }

    #[test]
    fn 没有文件和格式不对分开报() {
        let directory =
            std::env::temp_dir().join(format!("agent-room-ip-country-{}", std::process::id()));
        assert!(matches!(
            MaxMindCountryLookup::open(&directory.join("missing.mmdb")),
            Err(CountryDatabaseError::Missing)
        ));
        assert!(matches!(
            MaxMindCountryLookup::from_bytes(b"not a database".to_vec()),
            Err(CountryDatabaseError::Invalid(_))
        ));
    }
}
