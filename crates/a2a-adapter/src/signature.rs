use std::{collections::BTreeMap, sync::Arc};

use agent_room_domain::agent_cards::AgentCardSourceUrl;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{
    Algorithm, DecodingKey,
    crypto::verify,
    jwk::{Jwk, JwkSet, KeyOperations, PublicKeyUse},
};
use serde::Deserialize;
use serde_json::{Map, Value};
use thiserror::Error;
use url::Url;

use crate::{
    HttpsDocumentClient, NetworkTargetFailureKind, ParsedAgentCard, wire::WireAgentCardSignature,
};

const MAXIMUM_PROTECTED_HEADER_BYTES: usize = 4_096;
const MAXIMUM_JWKS_BYTES: usize = 65_536;
const MAXIMUM_JWKS_KEYS: usize = 32;
const MAXIMUM_KEY_ID_LENGTH: usize = 256;
const PROTECTED_HEADER_NAMES: [&str; 9] = [
    "alg", "kid", "typ", "jku", "crit", "b64", "jwk", "x5c", "x5u",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCardSignatureFailureKind {
    InvalidSignature,
    BlockedNetworkTarget,
    Unavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Agent Card 验签操作 {operation} 失败：{kind:?}")]
pub struct AgentCardSignatureFailure {
    operation: &'static str,
    kind: AgentCardSignatureFailureKind,
}

impl AgentCardSignatureFailure {
    const fn new(operation: &'static str, kind: AgentCardSignatureFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> AgentCardSignatureFailureKind {
        self.kind
    }
}

type SignatureResult<T> = Result<T, AgentCardSignatureFailure>;

#[derive(Debug, Deserialize)]
struct ProtectedHeader {
    alg: String,
    kid: String,
    #[serde(default)]
    typ: Option<String>,
    #[serde(default)]
    jku: Option<String>,
    #[serde(default)]
    crit: Vec<String>,
    #[serde(default)]
    b64: Option<bool>,
    #[serde(flatten)]
    additional: BTreeMap<String, Value>,
}

pub struct AgentCardSignatureVerifier {
    documents: Arc<dyn HttpsDocumentClient>,
}

impl AgentCardSignatureVerifier {
    pub fn new(documents: Arc<dyn HttpsDocumentClient>) -> Self {
        Self { documents }
    }

    /// 验证至少一个 Agent Card JWS 签名。
    ///
    /// 多签名用于密钥轮换，因此单个旧签名失效不会覆盖另一个有效签名。
    ///
    /// # Errors
    ///
    /// 所有签名均无效、密钥来源不安全、JWKS 不可用或内部规范化失败时返回结构化错误。
    pub async fn verify(
        &self,
        parsed: &ParsedAgentCard,
        source_url: &AgentCardSourceUrl,
    ) -> SignatureResult<()> {
        if parsed.signatures().is_empty() {
            return Err(invalid_signature("a2a.agent_card.verify.no_signature"));
        }
        let payload = canonical_payload(parsed.raw())?;
        let mut final_failure = invalid_signature("a2a.agent_card.verify");
        for signature in parsed.signatures() {
            match self.verify_one(signature, &payload, source_url).await {
                Ok(()) => return Ok(()),
                Err(failure) => {
                    final_failure = prefer_infrastructure_failure(final_failure, failure);
                }
            }
        }
        Err(final_failure)
    }

    async fn verify_one(
        &self,
        signature: &WireAgentCardSignature,
        payload: &[u8],
        source_url: &AgentCardSourceUrl,
    ) -> SignatureResult<()> {
        let header = decode_protected_header(signature)?;
        let algorithm = parse_public_key_algorithm(&header.alg)?;
        validate_protected_header(&header)?;
        validate_unprotected_header(signature.header.as_ref())?;
        let jku = header
            .jku
            .as_deref()
            .ok_or_else(|| invalid_signature("a2a.agent_card.verify.missing_jku"))?;
        let key_source = AgentCardSourceUrl::new(jku.to_owned())
            .map_err(|_| invalid_signature("a2a.agent_card.verify.invalid_jku"))?;
        ensure_same_origin(source_url, &key_source)?;
        let document = self
            .documents
            .get_json(&key_source, MAXIMUM_JWKS_BYTES)
            .await
            .map_err(map_network_failure)?;
        let key_set = serde_json::from_slice::<JwkSet>(document.body())
            .map_err(|_| invalid_signature("a2a.agent_card.verify.invalid_jwks"))?;
        let key = select_key(&key_set, &header.kid, algorithm)?;
        let decoding_key = DecodingKey::from_jwk(key)
            .map_err(|_| invalid_signature("a2a.agent_card.verify.invalid_jwk"))?;
        if decoding_key.family() != algorithm.family() {
            return Err(invalid_signature("a2a.agent_card.verify.key_family"));
        }
        let encoded_payload = URL_SAFE_NO_PAD.encode(payload);
        let signing_input = format!("{}.{}", signature.protected, encoded_payload);
        let valid = verify(
            &signature.signature,
            signing_input.as_bytes(),
            &decoding_key,
            algorithm,
        )
        .map_err(|_| invalid_signature("a2a.agent_card.verify.crypto"))?;
        if valid {
            Ok(())
        } else {
            Err(invalid_signature("a2a.agent_card.verify.mismatch"))
        }
    }
}

fn decode_protected_header(signature: &WireAgentCardSignature) -> SignatureResult<ProtectedHeader> {
    if signature.protected.is_empty()
        || signature.protected.len() > MAXIMUM_PROTECTED_HEADER_BYTES.saturating_mul(2)
    {
        return Err(invalid_signature("a2a.agent_card.verify.protected_length"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(&signature.protected)
        .map_err(|_| invalid_signature("a2a.agent_card.verify.protected_encoding"))?;
    if bytes.len() > MAXIMUM_PROTECTED_HEADER_BYTES {
        return Err(invalid_signature("a2a.agent_card.verify.protected_length"));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| invalid_signature("a2a.agent_card.verify.protected_json"))
}

fn validate_protected_header(header: &ProtectedHeader) -> SignatureResult<()> {
    if header.kid.is_empty() || header.kid.len() > MAXIMUM_KEY_ID_LENGTH {
        return Err(invalid_signature("a2a.agent_card.verify.kid"));
    }
    if header.typ.as_deref().is_some_and(|typ| typ != "JOSE")
        || !header.crit.is_empty()
        || header.b64 == Some(false)
        || header.additional.contains_key("jwk")
        || header.additional.contains_key("x5c")
        || header.additional.contains_key("x5u")
    {
        return Err(invalid_signature("a2a.agent_card.verify.header_policy"));
    }
    Ok(())
}

fn validate_unprotected_header(header: Option<&Value>) -> SignatureResult<()> {
    let Some(header) = header else {
        return Ok(());
    };
    let Some(header) = header.as_object() else {
        return Err(invalid_signature(
            "a2a.agent_card.verify.unprotected_header",
        ));
    };
    if PROTECTED_HEADER_NAMES
        .iter()
        .any(|name| header.contains_key(*name))
    {
        Err(invalid_signature(
            "a2a.agent_card.verify.unprotected_collision",
        ))
    } else {
        Ok(())
    }
}

fn parse_public_key_algorithm(value: &str) -> SignatureResult<Algorithm> {
    let algorithm = value
        .parse::<Algorithm>()
        .map_err(|_| invalid_signature("a2a.agent_card.verify.algorithm"))?;
    if matches!(
        algorithm,
        Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::EdDSA
    ) {
        Ok(algorithm)
    } else {
        Err(invalid_signature("a2a.agent_card.verify.algorithm"))
    }
}

fn select_key<'a>(
    key_set: &'a JwkSet,
    key_id: &str,
    algorithm: Algorithm,
) -> SignatureResult<&'a Jwk> {
    if key_set.keys.is_empty() || key_set.keys.len() > MAXIMUM_JWKS_KEYS {
        return Err(invalid_signature("a2a.agent_card.verify.jwks_size"));
    }
    let mut matches = key_set
        .keys
        .iter()
        .filter(|key| key.common.key_id.as_deref() == Some(key_id));
    let key = matches
        .next()
        .ok_or_else(|| invalid_signature("a2a.agent_card.verify.unknown_kid"))?;
    if matches.next().is_some() {
        return Err(invalid_signature("a2a.agent_card.verify.ambiguous_kid"));
    }
    if key
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|usage| usage != &PublicKeyUse::Signature)
    {
        return Err(invalid_signature("a2a.agent_card.verify.key_use"));
    }
    if key
        .common
        .key_operations
        .as_ref()
        .is_some_and(|operations| {
            !operations.contains(&KeyOperations::Verify)
                || operations
                    .iter()
                    .any(|operation| operation != &KeyOperations::Verify)
        })
    {
        return Err(invalid_signature("a2a.agent_card.verify.key_operations"));
    }
    if key
        .common
        .key_algorithm
        .is_some_and(|key_algorithm| Algorithm::try_from(key_algorithm).ok() != Some(algorithm))
    {
        return Err(invalid_signature("a2a.agent_card.verify.key_algorithm"));
    }
    Ok(key)
}

fn ensure_same_origin(
    card_source: &AgentCardSourceUrl,
    key_source: &AgentCardSourceUrl,
) -> SignatureResult<()> {
    let card = Url::parse(card_source.as_str())
        .map_err(|_| invalid_signature("a2a.agent_card.verify.card_origin"))?;
    let key = Url::parse(key_source.as_str())
        .map_err(|_| invalid_signature("a2a.agent_card.verify.key_origin"))?;
    if card.scheme() == key.scheme()
        && card.host_str() == key.host_str()
        && card.port_or_known_default() == key.port_or_known_default()
    {
        Ok(())
    } else {
        Err(AgentCardSignatureFailure::new(
            "a2a.agent_card.verify.key_origin",
            AgentCardSignatureFailureKind::BlockedNetworkTarget,
        ))
    }
}

fn canonical_payload(raw: &Value) -> SignatureResult<Vec<u8>> {
    let mut payload = raw.clone();
    let root = payload
        .as_object_mut()
        .ok_or_else(|| invalid_signature("a2a.agent_card.verify.payload_shape"))?;
    root.remove("signatures");
    remove_empty_container(root, "securitySchemes");
    remove_empty_container(root, "securityRequirements");
    normalize_capabilities(root.get_mut("capabilities"));
    normalize_collection(root.get_mut("skills"), normalize_skill);
    normalize_collection(
        root.get_mut("securityRequirements"),
        normalize_security_requirement,
    );
    normalize_security_schemes(root.get_mut("securitySchemes"));
    serde_jcs::to_vec(&payload).map_err(|_| {
        AgentCardSignatureFailure::new(
            "a2a.agent_card.verify.canonicalize",
            AgentCardSignatureFailureKind::Internal,
        )
    })
}

fn normalize_capabilities(value: Option<&mut Value>) {
    let Some(object) = value.and_then(Value::as_object_mut) else {
        return;
    };
    remove_empty_container(object, "extensions");
    normalize_collection(object.get_mut("extensions"), normalize_extension);
}

fn normalize_extension(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    remove_default_string(object, "description");
    remove_default_bool(object, "required");
    remove_empty_container(object, "params");
}

fn normalize_skill(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for field in [
        "examples",
        "inputModes",
        "outputModes",
        "securityRequirements",
    ] {
        remove_empty_container(object, field);
    }
    normalize_collection(
        object.get_mut("securityRequirements"),
        normalize_security_requirement,
    );
}

fn normalize_security_requirement(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    remove_empty_container(object, "schemes");
    if let Some(schemes) = object.get_mut("schemes").and_then(Value::as_object_mut) {
        for scopes in schemes.values_mut() {
            let Some(scopes) = scopes.as_object_mut() else {
                continue;
            };
            remove_empty_container(scopes, "list");
        }
    }
}

fn normalize_security_schemes(value: Option<&mut Value>) {
    let Some(schemes) = value.and_then(Value::as_object_mut) else {
        return;
    };
    for scheme in schemes.values_mut() {
        normalize_default_tree(scheme);
    }
}

fn normalize_default_tree(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                normalize_default_tree(value);
            }
        }
        Value::Object(object) => {
            for value in object.values_mut() {
                normalize_default_tree(value);
            }
            object.retain(|_, value| !is_default_value(value));
        }
        _ => {}
    }
}

fn normalize_collection(value: Option<&mut Value>, normalize_item: fn(&mut Value)) {
    let Some(values) = value.and_then(Value::as_array_mut) else {
        return;
    };
    for value in values {
        normalize_item(value);
    }
}

fn remove_empty_container(object: &mut Map<String, Value>, field: &str) {
    if object.get(field).is_some_and(is_empty_container) {
        object.remove(field);
    }
}

fn remove_default_string(object: &mut Map<String, Value>, field: &str) {
    if object
        .get(field)
        .is_some_and(|value| value.as_str() == Some(""))
    {
        object.remove(field);
    }
}

fn remove_default_bool(object: &mut Map<String, Value>, field: &str) {
    if object
        .get(field)
        .is_some_and(|value| value.as_bool() == Some(false))
    {
        object.remove(field);
    }
}

fn is_empty_container(value: &Value) -> bool {
    value.as_array().is_some_and(Vec::is_empty) || value.as_object().is_some_and(Map::is_empty)
}

fn is_default_value(value: &Value) -> bool {
    value.is_null()
        || value.as_str() == Some("")
        || value.as_bool() == Some(false)
        || value.as_i64() == Some(0)
        || value.as_u64() == Some(0)
        || value.as_f64() == Some(0.0)
        || is_empty_container(value)
}

fn map_network_failure(failure: crate::NetworkTargetFailure) -> AgentCardSignatureFailure {
    let kind = match failure.kind() {
        NetworkTargetFailureKind::BlockedAddress | NetworkTargetFailureKind::InvalidTarget => {
            AgentCardSignatureFailureKind::BlockedNetworkTarget
        }
        NetworkTargetFailureKind::ResolutionFailed | NetworkTargetFailureKind::ConnectFailed => {
            AgentCardSignatureFailureKind::Unavailable
        }
        NetworkTargetFailureKind::InvalidResponse | NetworkTargetFailureKind::ResponseTooLarge => {
            AgentCardSignatureFailureKind::InvalidSignature
        }
        NetworkTargetFailureKind::Internal => AgentCardSignatureFailureKind::Internal,
    };
    AgentCardSignatureFailure::new("a2a.agent_card.verify.fetch_jwks", kind)
}

const fn prefer_infrastructure_failure(
    current: AgentCardSignatureFailure,
    candidate: AgentCardSignatureFailure,
) -> AgentCardSignatureFailure {
    if signature_failure_priority(candidate.kind()) > signature_failure_priority(current.kind()) {
        candidate
    } else {
        current
    }
}

const fn signature_failure_priority(kind: AgentCardSignatureFailureKind) -> u8 {
    match kind {
        AgentCardSignatureFailureKind::InvalidSignature => 0,
        AgentCardSignatureFailureKind::Unavailable => 1,
        AgentCardSignatureFailureKind::BlockedNetworkTarget => 2,
        AgentCardSignatureFailureKind::Internal => 3,
    }
}

const fn invalid_signature(operation: &'static str) -> AgentCardSignatureFailure {
    AgentCardSignatureFailure::new(operation, AgentCardSignatureFailureKind::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, sync::Arc};

    use agent_room_application::ports::PortFuture;
    use agent_room_domain::{agent_cards::AgentCardSourceUrl, time::DurationMillis};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use ed25519_dalek::{Signer as _, SigningKey};
    use serde_json::{Value, json};

    use super::{AgentCardSignatureFailureKind, AgentCardSignatureVerifier, canonical_payload};
    use crate::{AgentCardNormalizer, HttpsDocumentClient, JsonDocument, NetworkTargetResult};

    struct FixedDocumentClient {
        document: JsonDocument,
    }

    impl HttpsDocumentClient for FixedDocumentClient {
        fn get_json<'a>(
            &'a self,
            _source_url: &'a AgentCardSourceUrl,
            _maximum_bytes: usize,
        ) -> PortFuture<'a, NetworkTargetResult<JsonDocument>> {
            let document = self.document.clone();
            Box::pin(async move { Ok(document) })
        }
    }

    #[tokio::test]
    async fn 同源_jwks_中的_ed25519_签名可以验证() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let key_id = "fixture-key";
        let raw = signed_fixture(&signing_key, key_id);
        let parsed = AgentCardNormalizer::new(BTreeSet::new())
            .parse(&document(&raw), &source_url())
            .expect("签名 Fixture 的资料结构有效");
        let verifier = AgentCardSignatureVerifier::new(Arc::new(FixedDocumentClient {
            document: document(&jwks(&signing_key, key_id)),
        }));

        verifier
            .verify(&parsed, &source_url())
            .await
            .expect("有效签名必须通过");
    }

    #[tokio::test]
    async fn 签名后的能力资料被篡改时明确拒绝() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let key_id = "fixture-key";
        let mut raw = signed_fixture(&signing_key, key_id);
        raw.as_object_mut()
            .expect("测试 Agent Card 是对象")
            .insert("name".to_owned(), json!("被篡改的名称"));
        let parsed = AgentCardNormalizer::default()
            .parse(&document(&raw), &source_url())
            .expect("被篡改 Card 的结构仍然有效");
        let verifier = AgentCardSignatureVerifier::new(Arc::new(FixedDocumentClient {
            document: document(&jwks(&signing_key, key_id)),
        }));

        let failure = verifier
            .verify(&parsed, &source_url())
            .await
            .expect_err("签名后的资料被篡改必须拒绝");

        assert_eq!(
            failure.kind(),
            AgentCardSignatureFailureKind::InvalidSignature
        );
    }

    #[tokio::test]
    async fn 跨域_jwks_在发起网络请求前被拒绝() {
        let mut raw = fixture_value();
        let protected = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "alg": "EdDSA",
                "kid": "fixture-key",
                "jku": "https://keys.attacker.example/jwks.json",
            }))
            .expect("测试头部可序列化"),
        );
        raw.as_object_mut().expect("测试 Agent Card 是对象").insert(
            "signatures".to_owned(),
            json!([{"protected": protected, "signature": "invalid"}]),
        );
        let parsed = AgentCardNormalizer::default()
            .parse(&document(&raw), &source_url())
            .expect("签名 Fixture 的资料结构有效");
        let verifier = AgentCardSignatureVerifier::new(Arc::new(FixedDocumentClient {
            document: document(&json!({"keys": []})),
        }));

        let failure = verifier
            .verify(&parsed, &source_url())
            .await
            .expect_err("跨域 JKU 必须拒绝");

        assert_eq!(
            failure.kind(),
            AgentCardSignatureFailureKind::BlockedNetworkTarget
        );
    }

    #[test]
    fn 签名载荷移除签名字段和非必需默认值() {
        let mut raw = fixture_value();
        raw.as_object_mut()
            .expect("测试 Agent Card 是对象")
            .extend([
                ("signatures".to_owned(), json!([])),
                ("securityRequirements".to_owned(), json!([])),
            ]);
        let payload = canonical_payload(&raw).expect("测试 Agent Card 可规范化");
        let payload = serde_json::from_slice::<Value>(&payload).expect("规范化结果仍是 JSON");

        assert!(payload.get("signatures").is_none());
        assert!(payload.get("securityRequirements").is_none());
        assert_eq!(payload["capabilities"]["pushNotifications"], false);
    }

    fn fixture_value() -> Value {
        serde_json::from_str(include_str!("../fixtures/a2a-1.0-agent-card.json"))
            .expect("测试 Fixture 有效")
    }

    fn signed_fixture(signing_key: &SigningKey, key_id: &str) -> Value {
        let mut raw = fixture_value();
        let protected = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "alg": "EdDSA",
                "typ": "JOSE",
                "kid": key_id,
                "jku": "https://agent.example/.well-known/jwks.json",
            }))
            .expect("测试头部可序列化"),
        );
        let payload = canonical_payload(&raw).expect("测试 Agent Card 可规范化");
        let encoded_payload = URL_SAFE_NO_PAD.encode(payload);
        let signing_input = format!("{protected}.{encoded_payload}");
        let signature = signing_key.sign(signing_input.as_bytes());
        raw.as_object_mut().expect("测试 Agent Card 是对象").insert(
            "signatures".to_owned(),
            json!([{
                "protected": protected,
                "signature": URL_SAFE_NO_PAD.encode(signature.to_bytes()),
            }]),
        );
        raw
    }

    fn jwks(signing_key: &SigningKey, key_id: &str) -> Value {
        json!({
            "keys": [{
                "kty": "OKP",
                "crv": "Ed25519",
                "x": URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes()),
                "kid": key_id,
                "use": "sig",
                "key_ops": ["verify"],
                "alg": "EdDSA"
            }]
        })
    }

    fn document(value: &Value) -> JsonDocument {
        JsonDocument::new(
            serde_json::to_vec(value).expect("测试 JSON 可序列化"),
            DurationMillis::new(60_000).expect("测试缓存时限有效"),
        )
    }

    fn source_url() -> AgentCardSourceUrl {
        AgentCardSourceUrl::new("https://agent.example/.well-known/agent-card.json".to_owned())
            .expect("测试来源有效")
    }
}
