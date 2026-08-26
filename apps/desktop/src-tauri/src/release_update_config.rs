use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use url::Url;

use agent_room_release_manifest::TrustedReleaseKey;

#[derive(Clone, Debug)]
pub(crate) struct ReleaseUpdateConfig {
    trusted_key: TrustedReleaseKey,
    tauri_public_key: String,
    stable_url: Url,
    testing_url: Url,
}

impl ReleaseUpdateConfig {
    pub(crate) fn from_build() -> Result<Option<Self>, ReleaseUpdateConfigFailure> {
        Self::from_values(BuildValues {
            required: option_env!("AGENT_ROOM_SIGNED_UPDATES_REQUIRED"),
            release_key_id: option_env!("AGENT_ROOM_RELEASE_KEY_ID"),
            release_public_key: option_env!("AGENT_ROOM_RELEASE_PUBLIC_KEY"),
            tauri_public_key: option_env!("AGENT_ROOM_TAURI_UPDATER_PUBLIC_KEY"),
            stable_url: option_env!("AGENT_ROOM_RELEASE_STABLE_URL"),
            testing_url: option_env!("AGENT_ROOM_RELEASE_TESTING_URL"),
        })
    }

    pub(crate) const fn trusted_key(&self) -> &TrustedReleaseKey {
        &self.trusted_key
    }

    pub(crate) fn tauri_public_key(&self) -> &str {
        &self.tauri_public_key
    }

    pub(crate) const fn stable_url(&self) -> &Url {
        &self.stable_url
    }

    pub(crate) const fn testing_url(&self) -> &Url {
        &self.testing_url
    }

    fn from_values(values: BuildValues<'_>) -> Result<Option<Self>, ReleaseUpdateConfigFailure> {
        let configured = [
            values.release_key_id,
            values.release_public_key,
            values.tauri_public_key,
            values.stable_url,
            values.testing_url,
        ];
        let present = configured.iter().filter(|value| value.is_some()).count();
        if present == 0 {
            return if values.required == Some("1") {
                Err(ReleaseUpdateConfigFailure::Missing)
            } else {
                Ok(None)
            };
        }
        if present != configured.len() {
            return Err(ReleaseUpdateConfigFailure::Partial);
        }

        let key_id = bounded(values.release_key_id.expect("完整配置已经校验"))?;
        let public_key = URL_SAFE_NO_PAD
            .decode(bounded(
                values.release_public_key.expect("完整配置已经校验"),
            )?)
            .map_err(|_| ReleaseUpdateConfigFailure::PublicKey)?;
        let public_key: [u8; 32] = public_key
            .try_into()
            .map_err(|_| ReleaseUpdateConfigFailure::PublicKey)?;
        ed25519_dalek::VerifyingKey::from_bytes(&public_key)
            .map_err(|_| ReleaseUpdateConfigFailure::PublicKey)?;
        let tauri_public_key =
            bounded(values.tauri_public_key.expect("完整配置已经校验"))?.to_owned();
        minisign_verify::PublicKey::from_base64(&tauri_public_key)
            .map_err(|_| ReleaseUpdateConfigFailure::PublicKey)?;

        Ok(Some(Self {
            trusted_key: TrustedReleaseKey {
                key_id: key_id.to_owned(),
                public_key,
            },
            tauri_public_key,
            stable_url: secure_url(values.stable_url.expect("完整配置已经校验"))?,
            testing_url: secure_url(values.testing_url.expect("完整配置已经校验"))?,
        }))
    }
}

#[derive(Clone, Copy)]
struct BuildValues<'a> {
    required: Option<&'a str>,
    release_key_id: Option<&'a str>,
    release_public_key: Option<&'a str>,
    tauri_public_key: Option<&'a str>,
    stable_url: Option<&'a str>,
    testing_url: Option<&'a str>,
}

fn bounded(value: &str) -> Result<&str, ReleaseUpdateConfigFailure> {
    let value = value.trim();
    if value.is_empty() || value.len() > 4_096 || value.chars().any(char::is_control) {
        return Err(ReleaseUpdateConfigFailure::Value);
    }
    Ok(value)
}

fn secure_url(value: &str) -> Result<Url, ReleaseUpdateConfigFailure> {
    let url = Url::parse(bounded(value)?).map_err(|_| ReleaseUpdateConfigFailure::Url)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(ReleaseUpdateConfigFailure::Url);
    }
    Ok(url)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReleaseUpdateConfigFailure {
    Missing,
    Partial,
    PublicKey,
    Url,
    Value,
}

impl ReleaseUpdateConfigFailure {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::Missing => "desktop.update.config_missing",
            Self::Partial => "desktop.update.config_partial",
            Self::PublicKey => "desktop.update.public_key_invalid",
            Self::Url => "desktop.update.endpoint_invalid",
            Self::Value => "desktop.update.config_value_invalid",
        }
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    use super::*;

    fn complete_values(public_key: &str) -> BuildValues<'_> {
        BuildValues {
            required: Some("1"),
            release_key_id: Some("release-2026"),
            release_public_key: Some(public_key),
            tauri_public_key: Some("RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3"),
            stable_url: Some("https://releases.example/stable.signed.json"),
            testing_url: Some("https://releases.example/testing.signed.json"),
        }
    }

    #[test]
    fn 完整配置会生成两个隔离渠道() {
        let public_key = URL_SAFE_NO_PAD.encode(
            ed25519_dalek::SigningKey::from_bytes(&[7; 32])
                .verifying_key()
                .to_bytes(),
        );
        let values = complete_values(&public_key);

        let config = ReleaseUpdateConfig::from_values(values)
            .expect("完整配置必须有效")
            .expect("必须启用更新");

        assert_eq!(config.stable_url().path(), "/stable.signed.json");
        assert_eq!(config.testing_url().path(), "/testing.signed.json");
    }

    #[test]
    fn 要求签名更新时拒绝缺失或部分配置() {
        assert!(matches!(
            ReleaseUpdateConfig::from_values(BuildValues {
                required: Some("1"),
                release_key_id: None,
                release_public_key: None,
                tauri_public_key: None,
                stable_url: None,
                testing_url: None,
            }),
            Err(ReleaseUpdateConfigFailure::Missing)
        ));
        let public_key = URL_SAFE_NO_PAD.encode(
            ed25519_dalek::SigningKey::from_bytes(&[7; 32])
                .verifying_key()
                .to_bytes(),
        );
        let mut partial = complete_values(&public_key);
        partial.testing_url = None;
        assert!(matches!(
            ReleaseUpdateConfig::from_values(partial),
            Err(ReleaseUpdateConfigFailure::Partial)
        ));
    }

    #[test]
    fn 拒绝非_https_渠道() {
        let public_key = URL_SAFE_NO_PAD.encode(
            ed25519_dalek::SigningKey::from_bytes(&[7; 32])
                .verifying_key()
                .to_bytes(),
        );
        let mut values = complete_values(&public_key);
        values.stable_url = Some("http://releases.example/stable.json");
        assert!(matches!(
            ReleaseUpdateConfig::from_values(values),
            Err(ReleaseUpdateConfigFailure::Url)
        ));
    }
}
