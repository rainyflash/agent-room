use agent_room_domain::{identity::Principal, ids::PrincipalId, time::UtcMillis};

use crate::ports::{PrincipalRegistration, ProfileImportConsent, VerifiedOidcIdentity};

const DEFAULT_LOCALE: &str = "en";

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
        matrix_user_id: format!("@user-{compact_id}:{matrix_server_name}"),
        display_name,
        avatar_content_id: None,
        locale,
        registered_at,
    }
}
