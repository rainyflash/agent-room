use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

pub(crate) const MIN_RANDOM_VALUE_LENGTH: usize = 32;
pub(crate) const MAX_RANDOM_VALUE_LENGTH: usize = 128;
pub(crate) const MAX_RETURN_PATH_LENGTH: usize = 2_048;

pub(crate) fn generate_random_url_safe_value() -> Result<String, getrandom::Error> {
    let mut entropy = [0_u8; 32];
    getrandom::fill(&mut entropy)?;
    Ok(URL_SAFE_NO_PAD.encode(entropy))
}

pub(crate) fn is_valid_random_url_safe_value(value: &str) -> bool {
    (MIN_RANDOM_VALUE_LENGTH..=MAX_RANDOM_VALUE_LENGTH).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub(crate) fn is_valid_return_path(value: &str) -> bool {
    value.starts_with('/')
        && !value.starts_with("//")
        && value.len() <= MAX_RETURN_PATH_LENGTH
        && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::{
        generate_random_url_safe_value, is_valid_random_url_safe_value, is_valid_return_path,
    };

    #[test]
    fn 返回路径只接受站内绝对路径() {
        assert!(is_valid_return_path("/lobby/public?view=scene"));
        assert!(!is_valid_return_path("https://evil.example"));
        assert!(!is_valid_return_path("//evil.example"));
        assert!(!is_valid_return_path("/lobby\n/injected"));
    }

    #[test]
    fn 随机值满足_url_safe_边界() {
        let value = generate_random_url_safe_value().expect("系统熵可用");
        assert!(is_valid_random_url_safe_value(&value));
    }
}
