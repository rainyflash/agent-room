//! 粗地区（`specs/measurement/design.md` 第 3 步）：只用来按地区汇总有多少账户在用、
//! 页面和接口有多快，不按人记。地区按以后要做的决定来分，中国大陆单列。
//!
//! 国家到地区的对照写死在下面的表里，每个国家只在一个地区里。没有单列的放进离它最近的那个：
//! 俄罗斯、白俄罗斯、外高加索、中亚和土耳其算欧洲（网络多半经莫斯科、法兰克福、伊斯坦布尔走），
//! 蒙古和朝鲜算日韩，阿富汗和英属印度洋领地算南亚，南极洲和澳新的海外领地算大洋洲，
//! 格陵兰算北美，加勒比海的美国、英国、法国和荷兰属地算拉美。

/// 一个粗地区。指标标签只用 [`Region::slug`]，取值固定这十一个。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    MainlandChina,
    HongKongMacauTaiwan,
    JapanKorea,
    SoutheastAsia,
    SouthAsia,
    Oceania,
    Europe,
    NorthAmerica,
    LatinAmerica,
    MiddleEastAfrica,
    /// 查不出来：没有 IP 库、地址不在库里、请求没经过网关。
    Unknown,
}

impl Region {
    pub const ALL: [Self; 11] = [
        Self::MainlandChina,
        Self::HongKongMacauTaiwan,
        Self::JapanKorea,
        Self::SoutheastAsia,
        Self::SouthAsia,
        Self::Oceania,
        Self::Europe,
        Self::NorthAmerica,
        Self::LatinAmerica,
        Self::MiddleEastAfrica,
        Self::Unknown,
    ];

    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::MainlandChina => "cn",
            Self::HongKongMacauTaiwan => "hk_mo_tw",
            Self::JapanKorea => "jp_kr",
            Self::SoutheastAsia => "southeast_asia",
            Self::SouthAsia => "south_asia",
            Self::Oceania => "oceania",
            Self::Europe => "europe",
            Self::NorthAmerica => "north_america",
            Self::LatinAmerica => "latin_america",
            Self::MiddleEastAfrica => "middle_east_africa",
            Self::Unknown => "unknown",
        }
    }

    /// 国家所在的地区；表里没有的算未知。
    #[must_use]
    pub fn of_country(country: CountryCode) -> Self {
        COUNTRIES
            .iter()
            .find(|(_, codes)| codes.contains(&country.as_str()))
            .map_or(Self::Unknown, |(region, _)| *region)
    }
}

/// ISO 3166-1 的两个字母（另加 IP 库给科索沃用的 `XK`），存成大写。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CountryCode([u8; 2]);

impl CountryCode {
    /// 只认两个 ASCII 字母，不分大小写。
    #[must_use]
    pub fn parse(code: &str) -> Option<Self> {
        match code.as_bytes() {
            [first, second] if first.is_ascii_alphabetic() && second.is_ascii_alphabetic() => {
                Some(Self([
                    first.to_ascii_uppercase(),
                    second.to_ascii_uppercase(),
                ]))
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        // 构造时只收 ASCII 字母。
        std::str::from_utf8(&self.0).unwrap_or_default()
    }
}

const COUNTRIES: [(Region, &[&str]); 10] = [
    (Region::MainlandChina, &["CN"]),
    (Region::HongKongMacauTaiwan, &["HK", "MO", "TW"]),
    (Region::JapanKorea, &["JP", "KP", "KR", "MN"]),
    (
        Region::SoutheastAsia,
        &[
            "BN", "ID", "KH", "LA", "MM", "MY", "PH", "SG", "TH", "TL", "VN",
        ],
    ),
    (
        Region::SouthAsia,
        &["AF", "BD", "BT", "IN", "IO", "LK", "MV", "NP", "PK"],
    ),
    (
        Region::Oceania,
        &[
            "AQ", "AS", "AU", "CC", "CK", "CX", "FJ", "FM", "GU", "HM", "KI", "MH", "MP", "NC",
            "NF", "NR", "NU", "NZ", "PF", "PG", "PN", "PW", "SB", "TK", "TO", "TV", "UM", "VU",
            "WF", "WS",
        ],
    ),
    (
        Region::Europe,
        &[
            "AD", "AL", "AM", "AT", "AX", "AZ", "BA", "BE", "BG", "BY", "CH", "CY", "CZ", "DE",
            "DK", "EE", "ES", "FI", "FO", "FR", "GB", "GE", "GG", "GI", "GR", "HR", "HU", "IE",
            "IM", "IS", "IT", "JE", "KG", "KZ", "LI", "LT", "LU", "LV", "MC", "MD", "ME", "MK",
            "MT", "NL", "NO", "PL", "PT", "RO", "RS", "RU", "SE", "SI", "SJ", "SK", "SM", "TJ",
            "TM", "TR", "UA", "UZ", "VA", "XK",
        ],
    ),
    (Region::NorthAmerica, &["BM", "CA", "GL", "PM", "US"]),
    (
        Region::LatinAmerica,
        &[
            "AG", "AI", "AR", "AW", "BB", "BL", "BO", "BQ", "BR", "BS", "BZ", "CL", "CO", "CR",
            "CU", "CW", "DM", "DO", "EC", "FK", "GD", "GF", "GP", "GS", "GT", "GY", "HN", "HT",
            "JM", "KN", "KY", "LC", "MF", "MQ", "MS", "MX", "NI", "PA", "PE", "PR", "PY", "SR",
            "SV", "SX", "TC", "TT", "UY", "VC", "VE", "VG", "VI",
        ],
    ),
    (
        Region::MiddleEastAfrica,
        &[
            "AE", "AO", "BF", "BH", "BI", "BJ", "BV", "BW", "CD", "CF", "CG", "CI", "CM", "CV",
            "DJ", "DZ", "EG", "EH", "ER", "ET", "GA", "GH", "GM", "GN", "GQ", "GW", "IL", "IQ",
            "IR", "JO", "KE", "KM", "KW", "LB", "LR", "LS", "LY", "MA", "MG", "ML", "MR", "MU",
            "MW", "MZ", "NA", "NE", "NG", "OM", "PS", "QA", "RE", "RW", "SA", "SC", "SD", "SH",
            "SL", "SN", "SO", "SS", "ST", "SY", "SZ", "TD", "TF", "TG", "TN", "TZ", "UG", "YE",
            "YT", "ZA", "ZM", "ZW",
        ],
    ),
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{COUNTRIES, CountryCode, Region};

    fn region(code: &str) -> Region {
        Region::of_country(CountryCode::parse(code).expect("测试用的国家代码有效"))
    }

    #[test]
    fn 每个国家只在一个地区里_249_个国家加上科索沃都有地区() {
        let mut seen = HashSet::new();
        for (region, codes) in COUNTRIES {
            assert_ne!(region, Region::Unknown);
            for code in codes {
                assert!(
                    CountryCode::parse(code).is_some_and(|parsed| parsed.as_str() == *code),
                    "{code} 不是两个大写字母"
                );
                assert!(seen.insert(*code), "{code} 在两个地区里");
            }
        }
        assert_eq!(seen.len(), 250);
    }

    #[test]
    fn 表里每个地区都有国家_未知没有() {
        let listed: HashSet<_> = COUNTRIES.iter().map(|(region, _)| *region).collect();
        for region in Region::ALL {
            assert_eq!(
                listed.contains(&region),
                region != Region::Unknown,
                "{region:?}"
            );
        }
    }

    #[test]
    fn 中国大陆单列_港澳台另算() {
        assert_eq!(region("CN"), Region::MainlandChina);
        for code in ["HK", "MO", "TW"] {
            assert_eq!(region(code), Region::HongKongMacauTaiwan, "{code}");
        }
    }

    #[test]
    fn 没有单列的国家放进最近的地区() {
        for (code, expected) in [
            ("RU", Region::Europe),
            ("BY", Region::Europe),
            ("KZ", Region::Europe),
            ("UZ", Region::Europe),
            ("GE", Region::Europe),
            ("TR", Region::Europe),
            ("MN", Region::JapanKorea),
            ("AF", Region::SouthAsia),
            ("GL", Region::NorthAmerica),
            ("PR", Region::LatinAmerica),
            ("AQ", Region::Oceania),
        ] {
            assert_eq!(region(code), expected, "{code}");
        }
    }

    #[test]
    fn 常见国家落在对的地区() {
        for (code, expected) in [
            ("JP", Region::JapanKorea),
            ("KR", Region::JapanKorea),
            ("SG", Region::SoutheastAsia),
            ("VN", Region::SoutheastAsia),
            ("IN", Region::SouthAsia),
            ("AU", Region::Oceania),
            ("NZ", Region::Oceania),
            ("DE", Region::Europe),
            ("GB", Region::Europe),
            ("US", Region::NorthAmerica),
            ("CA", Region::NorthAmerica),
            ("MX", Region::LatinAmerica),
            ("BR", Region::LatinAmerica),
            ("AE", Region::MiddleEastAfrica),
            ("IL", Region::MiddleEastAfrica),
            ("EG", Region::MiddleEastAfrica),
            ("ZA", Region::MiddleEastAfrica),
        ] {
            assert_eq!(region(code), expected, "{code}");
        }
    }

    #[test]
    fn 国家代码不分大小写_不认的算未知() {
        assert_eq!(region("cn"), Region::MainlandChina);
        assert_eq!(region("ZZ"), Region::Unknown);
        assert_eq!(region("EU"), Region::Unknown);
        for invalid in ["", "C", "CHN", "1N", "中国"] {
            assert!(CountryCode::parse(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn 地区标签取值固定() {
        let slugs: Vec<_> = Region::ALL.iter().map(|region| region.slug()).collect();
        assert_eq!(
            slugs,
            [
                "cn",
                "hk_mo_tw",
                "jp_kr",
                "southeast_asia",
                "south_asia",
                "oceania",
                "europe",
                "north_america",
                "latin_america",
                "middle_east_africa",
                "unknown",
            ]
        );
    }
}
