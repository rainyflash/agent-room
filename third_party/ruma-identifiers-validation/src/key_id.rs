use std::num::NonZeroU8;

use crate::{Error, KeyName};

pub fn validate<K: KeyName + ?Sized>(s: &str) -> Result<NonZeroU8, Error> {
    // Agent Room 补丁（RUSTSEC-2026-0589）：原来先把冒号的位置 `as u8` 截断再切字符串，
    // 算法那一段超过 255 字节又带非 ASCII 字符时切到字符中间就 panic。现在照真的位置切，
    // 放不进返回值的当成太长。上游在 0.14.0 把返回值换成了 `()`，ruma-common 0.19 认的还是
    // `NonZeroU8`（它不用这个数，`KeyId` 自己再找冒号）。
    let colon_idx = s.find(':').ok_or(Error::MissingColon)?;
    let colon = NonZeroU8::new(u8::try_from(colon_idx).map_err(|_| Error::MaximumLengthExceeded)?)
        .ok_or(Error::MissingColon)?;

    K::validate(&s[colon_idx + 1..])?;

    Ok(colon)
}

#[cfg(test)]
mod tests {
    use super::validate;
    use crate::{Error, KeyName};

    struct AnyName;

    impl AsRef<str> for AnyName {
        fn as_ref(&self) -> &str {
            ""
        }
    }

    impl KeyName for AnyName {
        fn validate(_s: &str) -> Result<(), Error> {
            Ok(())
        }
    }

    #[test]
    fn returns_the_colon_index() {
        assert_eq!(validate::<AnyName>("ed25519:DEVICE").map(|index| index.get()), Ok(7));
        assert_eq!(validate::<AnyName>(":DEVICE"), Err(Error::MissingColon));
        assert_eq!(validate::<AnyName>("ed25519"), Err(Error::MissingColon));
    }

    #[test]
    fn long_non_ascii_algorithms_are_rejected_instead_of_panicking() {
        // 冒号在第 300 字节：截断成 u8 是 44，原来从 45 切，正好切在两字节的 "é" 中间。
        let key_id = format!("{}:DEVICE", "é".repeat(150));
        assert_eq!(validate::<AnyName>(&key_id), Err(Error::MaximumLengthExceeded));

        let key_id = format!("{}:DEVICE", "a".repeat(300));
        assert_eq!(validate::<AnyName>(&key_id), Err(Error::MaximumLengthExceeded));
    }
}
