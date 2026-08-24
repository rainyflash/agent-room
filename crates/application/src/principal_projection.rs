use agent_room_domain::{identity::Principal, ids::PrincipalId, time::UtcMillis};
use sha2::{Digest, Sha256};

use crate::ports::{PrincipalRegistration, ProfileImportConsent, VerifiedOidcIdentity};

const DEFAULT_LOCALE: &str = "en";
const MATRIX_LOCALPART_HASH_BYTES: usize = 16;

fn matrix_localpart(issuer: &str, subject: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(issuer.as_bytes());
    hasher.update([0]);
    hasher.update(subject.as_bytes());

    let digest = hasher.finalize();
    let mut localpart = String::with_capacity("user-".len() + MATRIX_LOCALPART_HASH_BYTES * 2);
    localpart.push_str("user-");
    for byte in digest.iter().take(MATRIX_LOCALPART_HASH_BYTES) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        localpart.push(char::from(HEX[usize::from(byte >> 4)]));
        localpart.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    localpart
}

pub(crate) fn principal_registration(
    id: PrincipalId,
    identity: &VerifiedOidcIdentity,
    consent: ProfileImportConsent,
    registered_at: UtcMillis,
    matrix_server_name: &str,
) -> PrincipalRegistration {
    let compact_id = id.to_string().replace('-', "");
    let default_display_name = format!("Agent Room User {}", &compact_id[..8]);
    let display_name = consent
        .display_name
        .then(|| identity.display_name())
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&default_display_name)
        .to_owned();
    let locale = consent
        .locale
        .then(|| identity.locale())
        .flatten()
        .unwrap_or(DEFAULT_LOCALE)
        .to_owned();

    PrincipalRegistration {
        principal: Principal::new(id),
        oidc_issuer: identity.issuer().to_owned(),
        oidc_subject: identity.subject().to_owned(),
        matrix_user_id: format!(
            "@{}:{matrix_server_name}",
            matrix_localpart(identity.issuer(), identity.subject())
        ),
        display_name,
        avatar_content_id: None,
        locale,
        registered_at,
    }
}

#[cfg(test)]
mod tests {
    use super::matrix_localpart;

    #[test]
    fn matrix_localpart_与_synapse_映射器共享固定测试向量() {
        assert_eq!(
            matrix_localpart("https://issuer.example", "subject"),
            "user-02e6095dff02265b8a8c5ab16314575c"
        );
    }

    #[test]
    fn matrix_localpart_同时绑定_issuer_与_subject() {
        let baseline = matrix_localpart("https://issuer.example", "subject");

        assert_ne!(
            baseline,
            matrix_localpart("https://other-issuer.example", "subject")
        );
        assert_ne!(
            baseline,
            matrix_localpart("https://issuer.example", "other-subject")
        );
    }
}
