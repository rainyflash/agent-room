use std::num::NonZeroU8;

use crate::{error::MxcUriError, server_name};

const PROTOCOL: &str = "mxc://";

pub fn validate(uri: &str) -> Result<NonZeroU8, MxcUriError> {
    let Some(uri) = uri.strip_prefix(PROTOCOL) else {
        return Err(MxcUriError::WrongSchema);
    };

    let Some(index) = uri.find('/') else {
        return Err(MxcUriError::MissingSlash);
    };

    let server_name = &uri[..index];
    let media_id = &uri[index + 1..];
    // See: https://spec.matrix.org/v1.18/client-server-api/#security-considerations-5
    let media_id_is_valid = media_id
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'-' | b'_' ));

    if !media_id_is_valid {
        Err(MxcUriError::MediaIdMalformed)
    } else if server_name::validate(server_name).is_err() {
        Err(MxcUriError::ServerNameMalformed)
    } else {
        // Agent Room 补丁（RUSTSEC-2026-0588）：原来是 `(index + 6) as u8` 再 `unwrap`，
        // 服务器名超过 249 字节时截断成 0 就 panic，截成别的数则让 `MxcUri::parts` 切错位置再 panic。
        // 上游在 0.13.0 把返回值换成 `NonZeroU16`、接受到 261 字节，ruma-common 0.19 认的是
        // `NonZeroU8`，所以这里放不下的一律当服务器名不合法。
        u8::try_from(index + PROTOCOL.len())
            .ok()
            .and_then(NonZeroU8::new)
            .ok_or(MxcUriError::ServerNameMalformed)
    }
}

#[cfg(test)]
mod tests {
    use super::validate;
    use crate::error::MxcUriError;

    #[test]
    fn returns_the_slash_index() {
        assert_eq!(validate("mxc://127.0.0.1/abc").map(|index| index.get()), Ok(15));
    }

    #[test]
    fn long_server_names_are_rejected_instead_of_panicking() {
        for length in [249, 250, 255, 256, 261] {
            let uri = format!("mxc://{}/1", "a".repeat(length));
            let expected = if length + 6 <= 255 {
                Ok(u8::try_from(length + 6).unwrap())
            } else {
                Err(MxcUriError::ServerNameMalformed)
            };
            assert_eq!(validate(&uri).map(|index| index.get()), expected, "{length}");
        }
    }
}
