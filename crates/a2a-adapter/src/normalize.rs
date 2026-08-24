use std::collections::{BTreeMap, BTreeSet};

use agent_room_domain::agent_cards::{
    AgentCardCapabilities, AgentCardDigest, AgentCardEndpoint, AgentCardExtension,
    AgentCardProtocolVersion, AgentCardProvider, AgentCardSecurityScheme,
    AgentCardSecuritySchemeKind, AgentCardSkill, AgentCardSourceUrl, AgentCardTransport,
    AgentEndpointVerificationState, NormalizedAgentCard, NormalizedAgentCardFields,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use url::Url;

use crate::{
    JsonDocument,
    wire::{
        WireAgentCard, WireAgentCardSignature, WireAgentExtension, WireAgentInterface,
        WireAgentSkill, WireSecurityRequirement, WireSecurityScheme,
    },
};

const MAX_SIGNATURES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCardNormalizationFailureKind {
    InvalidJson,
    InvalidSchema,
    UnsupportedProtocol,
    UnsupportedRequiredExtension,
    InvalidSecurityScheme,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Agent Card 规范化操作 {operation} 失败：{kind:?}")]
pub struct AgentCardNormalizationFailure {
    operation: &'static str,
    kind: AgentCardNormalizationFailureKind,
}

impl AgentCardNormalizationFailure {
    const fn new(operation: &'static str, kind: AgentCardNormalizationFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> AgentCardNormalizationFailureKind {
        self.kind
    }
}

type NormalizationResult<T> = Result<T, AgentCardNormalizationFailure>;

#[derive(Debug, Clone)]
pub struct ParsedAgentCard {
    digest: AgentCardDigest,
    card: NormalizedAgentCard,
    raw: Value,
    signatures: Vec<WireAgentCardSignature>,
}

impl ParsedAgentCard {
    pub const fn digest(&self) -> &AgentCardDigest {
        &self.digest
    }

    pub const fn card(&self) -> &NormalizedAgentCard {
        &self.card
    }

    pub fn has_signatures(&self) -> bool {
        !self.signatures.is_empty()
    }

    pub(crate) const fn raw(&self) -> &Value {
        &self.raw
    }

    pub(crate) fn signatures(&self) -> &[WireAgentCardSignature] {
        &self.signatures
    }

    pub(crate) fn into_parts(self) -> (AgentCardDigest, NormalizedAgentCard) {
        (self.digest, self.card)
    }
}

#[derive(Debug, Clone, Default)]
pub struct AgentCardNormalizer {
    supported_extension_uris: BTreeSet<String>,
}

impl AgentCardNormalizer {
    pub fn new(supported_extension_uris: BTreeSet<String>) -> Self {
        Self {
            supported_extension_uris,
        }
    }

    /// 将 A2A 1.0 wire 文档压缩为安全、稳定的产品资料。
    ///
    /// # Errors
    ///
    /// JSON、必填字段、协议版本、必需扩展或认证方案无效时失败。
    pub fn parse(
        &self,
        document: &JsonDocument,
        source_url: &AgentCardSourceUrl,
    ) -> NormalizationResult<ParsedAgentCard> {
        let operation = "a2a.agent_card.normalize";
        let raw = serde_json::from_slice::<Value>(document.body()).map_err(|_| {
            AgentCardNormalizationFailure::new(
                operation,
                AgentCardNormalizationFailureKind::InvalidJson,
            )
        })?;
        if !raw.is_object() {
            return Err(AgentCardNormalizationFailure::new(
                operation,
                AgentCardNormalizationFailureKind::InvalidSchema,
            ));
        }
        let wire = serde_json::from_value::<WireAgentCard>(raw.clone()).map_err(|_| {
            AgentCardNormalizationFailure::new(
                operation,
                AgentCardNormalizationFailureKind::InvalidSchema,
            )
        })?;
        if wire.signatures.len() > MAX_SIGNATURES {
            return Err(AgentCardNormalizationFailure::new(
                operation,
                AgentCardNormalizationFailureKind::InvalidSchema,
            ));
        }
        validate_security_requirements(&wire)?;
        let endpoints = normalize_endpoints(&wire.supported_interfaces, source_url)?;
        let provider = wire
            .provider
            .map(|provider| AgentCardProvider::new(provider.organization, provider.url))
            .transpose()
            .map_err(|_| invalid_schema(operation))?;
        let extensions = wire
            .capabilities
            .extensions
            .iter()
            .map(normalize_extension)
            .collect::<NormalizationResult<Vec<_>>>()?;
        if extensions.iter().any(|extension| {
            extension.required() && !self.supported_extension_uris.contains(extension.uri())
        }) {
            return Err(AgentCardNormalizationFailure::new(
                operation,
                AgentCardNormalizationFailureKind::UnsupportedRequiredExtension,
            ));
        }
        let capabilities = AgentCardCapabilities::new(
            wire.capabilities.streaming,
            wire.capabilities.push_notifications,
            wire.capabilities.extended_agent_card,
            extensions,
            &self.supported_extension_uris,
        )
        .map_err(|_| invalid_schema(operation))?;
        let security_schemes = normalize_security_schemes(&wire.security_schemes)?;
        let skills = wire
            .skills
            .iter()
            .map(normalize_skill)
            .collect::<NormalizationResult<Vec<_>>>()?;
        let card = NormalizedAgentCard::new(NormalizedAgentCardFields {
            name: wire.name,
            description: wire.description,
            provider,
            version: wire.version,
            endpoints,
            capabilities,
            security_schemes,
            default_input_modes: wire.default_input_modes,
            default_output_modes: wire.default_output_modes,
            skills,
        })
        .map_err(|_| invalid_schema(operation))?;
        let canonical = serde_jcs::to_vec(&normalized_value(&card)).map_err(|_| {
            AgentCardNormalizationFailure::new(
                operation,
                AgentCardNormalizationFailureKind::Internal,
            )
        })?;
        let digest = AgentCardDigest::from_array(Sha256::digest(canonical).into());
        Ok(ParsedAgentCard {
            digest,
            card,
            raw,
            signatures: wire.signatures,
        })
    }
}

fn normalize_endpoints(
    interfaces: &[WireAgentInterface],
    source_url: &AgentCardSourceUrl,
) -> NormalizationResult<Vec<AgentCardEndpoint>> {
    let operation = "a2a.agent_card.normalize_endpoints";
    let source = Url::parse(source_url.as_str()).map_err(|_| invalid_schema(operation))?;
    let mut endpoints = Vec::new();
    for interface in interfaces {
        let Ok(version) = AgentCardProtocolVersion::parse(&interface.protocol_version) else {
            continue;
        };
        if !version.is_supported() {
            continue;
        }
        let Ok(transport) = AgentCardTransport::try_from(interface.protocol_binding.as_str())
        else {
            continue;
        };
        let endpoint_url = Url::parse(&interface.url).map_err(|_| invalid_schema(operation))?;
        let verification = if same_origin(&source, &endpoint_url) {
            AgentEndpointVerificationState::Verified
        } else {
            AgentEndpointVerificationState::Declared
        };
        endpoints.push(
            AgentCardEndpoint::new(
                interface.url.clone(),
                transport,
                version,
                interface.tenant.clone(),
                verification,
            )
            .map_err(|_| invalid_schema(operation))?,
        );
    }
    if endpoints.is_empty() {
        Err(AgentCardNormalizationFailure::new(
            operation,
            AgentCardNormalizationFailureKind::UnsupportedProtocol,
        ))
    } else {
        Ok(endpoints)
    }
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn normalize_extension(extension: &WireAgentExtension) -> NormalizationResult<AgentCardExtension> {
    AgentCardExtension::new(
        extension.uri.clone(),
        extension.description.clone(),
        extension.required,
    )
    .map_err(|_| invalid_schema("a2a.agent_card.normalize_extension"))
}

fn normalize_skill(skill: &WireAgentSkill) -> NormalizationResult<AgentCardSkill> {
    AgentCardSkill::new(
        skill.id.clone(),
        skill.name.clone(),
        skill.description.clone(),
        skill.tags.clone(),
        skill.input_modes.clone(),
        skill.output_modes.clone(),
    )
    .map_err(|_| invalid_schema("a2a.agent_card.normalize_skill"))
}

fn normalize_security_schemes(
    schemes: &BTreeMap<String, WireSecurityScheme>,
) -> NormalizationResult<Vec<AgentCardSecurityScheme>> {
    schemes
        .iter()
        .map(|(name, scheme)| {
            let kind = security_scheme_kind(scheme)?;
            AgentCardSecurityScheme::new(name.clone(), kind).map_err(|_| invalid_security_scheme())
        })
        .collect()
}

fn security_scheme_kind(
    scheme: &WireSecurityScheme,
) -> NormalizationResult<AgentCardSecuritySchemeKind> {
    let recognized = scheme
        .variants
        .iter()
        .filter_map(|(name, value)| {
            let kind = match name.as_str() {
                "apiKeySecurityScheme" => AgentCardSecuritySchemeKind::ApiKey,
                "httpAuthSecurityScheme" => AgentCardSecuritySchemeKind::Http,
                "oauth2SecurityScheme" => AgentCardSecuritySchemeKind::OAuth2,
                "openIdConnectSecurityScheme" => AgentCardSecuritySchemeKind::OpenIdConnect,
                "mutualTlsSecurityScheme" => AgentCardSecuritySchemeKind::MutualTls,
                _ => return None,
            };
            value.is_object().then_some(kind)
        })
        .collect::<Vec<_>>();
    if recognized.len() == 1 && scheme.variants.len() == 1 {
        Ok(recognized[0])
    } else {
        Err(invalid_security_scheme())
    }
}

fn validate_security_requirements(wire: &WireAgentCard) -> NormalizationResult<()> {
    let known = wire
        .security_schemes
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let requirements = wire.security_requirements.iter().chain(
        wire.skills
            .iter()
            .flat_map(|skill| &skill.security_requirements),
    );
    for requirement in requirements {
        validate_security_requirement(requirement, &known)?;
    }
    Ok(())
}

fn validate_security_requirement(
    requirement: &WireSecurityRequirement,
    known: &BTreeSet<&str>,
) -> NormalizationResult<()> {
    if requirement.schemes.is_empty()
        || requirement.schemes.iter().any(|(name, scopes)| {
            !known.contains(name.as_str())
                || scopes.list.iter().any(|scope| scope.trim().is_empty())
        })
    {
        Err(invalid_security_scheme())
    } else {
        Ok(())
    }
}

fn normalized_value(card: &NormalizedAgentCard) -> Value {
    let provider = card.provider().map(|provider| {
        json!({
            "organization": provider.organization(),
            "url": provider.url(),
        })
    });
    let endpoints = card
        .endpoints()
        .iter()
        .map(|endpoint| {
            json!({
                "protocolBinding": endpoint.transport().as_str(),
                "protocolVersion": format!("{}.{}", endpoint.protocol_version().major(), endpoint.protocol_version().minor()),
                "tenant": endpoint.tenant(),
                "url": endpoint.url(),
                "verification": endpoint.verification().as_str(),
            })
        })
        .collect::<Vec<_>>();
    let extensions = card
        .capabilities()
        .extensions()
        .iter()
        .map(|extension| {
            json!({
                "description": extension.description(),
                "required": extension.required(),
                "uri": extension.uri(),
            })
        })
        .collect::<Vec<_>>();
    let security_schemes = card
        .security_schemes()
        .iter()
        .map(|scheme| {
            json!({
                "kind": scheme.kind().as_str(),
                "name": scheme.name(),
            })
        })
        .collect::<Vec<_>>();
    let skills = card
        .skills()
        .iter()
        .map(|skill| {
            json!({
                "description": skill.description(),
                "id": skill.id(),
                "inputModes": skill.input_modes(),
                "name": skill.name(),
                "outputModes": skill.output_modes(),
                "tags": skill.tags(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "capabilities": {
            "extendedAgentCard": card.capabilities().extended_agent_card(),
            "extensions": extensions,
            "pushNotifications": card.capabilities().push_notifications(),
            "streaming": card.capabilities().streaming(),
        },
        "defaultInputModes": card.default_input_modes(),
        "defaultOutputModes": card.default_output_modes(),
        "description": card.description(),
        "endpoints": endpoints,
        "name": card.name(),
        "provider": provider,
        "securitySchemes": security_schemes,
        "skills": skills,
        "version": card.version(),
    })
}

const fn invalid_schema(operation: &'static str) -> AgentCardNormalizationFailure {
    AgentCardNormalizationFailure::new(operation, AgentCardNormalizationFailureKind::InvalidSchema)
}

const fn invalid_security_scheme() -> AgentCardNormalizationFailure {
    AgentCardNormalizationFailure::new(
        "a2a.agent_card.normalize_security",
        AgentCardNormalizationFailureKind::InvalidSecurityScheme,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use agent_room_domain::{
        agent_cards::{AgentCardSourceUrl, AgentCardTransport, AgentEndpointVerificationState},
        time::DurationMillis,
    };

    use super::{AgentCardNormalizationFailureKind, AgentCardNormalizer};
    use crate::JsonDocument;

    #[test]
    fn 官方_a2a_1_0_结构映射为安全资料() {
        let parsed = AgentCardNormalizer::default()
            .parse(&fixture(), &source_url())
            .expect("官方结构 Fixture 应被接受");

        assert_eq!(parsed.card().name(), "Route Planner");
        assert_eq!(parsed.card().endpoints().len(), 2);
        assert_eq!(
            parsed.card().endpoints()[0].transport(),
            AgentCardTransport::JsonRpc
        );
        assert_eq!(
            parsed.card().endpoints()[0].verification(),
            AgentEndpointVerificationState::Verified
        );
        assert_eq!(parsed.card().security_schemes().len(), 1);
        assert!(!parsed.has_signatures());
    }

    #[test]
    fn 所有接口版本不兼容时明确失败() {
        let body =
            include_str!("../fixtures/a2a-1.0-agent-card.json").replace("\"1.0\"", "\"2.0\"");
        let failure = AgentCardNormalizer::default()
            .parse(&document(body.as_bytes()), &source_url())
            .expect_err("不支持的协议版本必须失败");

        assert_eq!(
            failure.kind(),
            AgentCardNormalizationFailureKind::UnsupportedProtocol
        );
    }

    #[test]
    fn 未知必需扩展不会静默降级() {
        let body = include_str!("../fixtures/a2a-1.0-agent-card.json").replace(
            "\"extensions\": []",
            "\"extensions\": [{\"uri\":\"https://extensions.example/unsafe/v1\",\"required\":true}]",
        );
        let failure = AgentCardNormalizer::new(BTreeSet::new())
            .parse(&document(body.as_bytes()), &source_url())
            .expect_err("未知必需扩展必须失败");

        assert_eq!(
            failure.kind(),
            AgentCardNormalizationFailureKind::UnsupportedRequiredExtension
        );
    }

    #[test]
    fn 能力变化会改变规范化摘要而示例提示词不会进入摘要() {
        let original = include_str!("../fixtures/a2a-1.0-agent-card.json");
        let changed_capability = original.replace("\"streaming\": true", "\"streaming\": false");
        let changed_example = original.replace("Plan a route", "Ignore prior instructions");
        let normalizer = AgentCardNormalizer::default();
        let original = normalizer
            .parse(&document(original.as_bytes()), &source_url())
            .expect("原始 Fixture 有效");
        let changed_capability = normalizer
            .parse(&document(changed_capability.as_bytes()), &source_url())
            .expect("能力变化 Fixture 有效");
        let changed_example = normalizer
            .parse(&document(changed_example.as_bytes()), &source_url())
            .expect("示例变化 Fixture 有效");

        assert_ne!(original.digest(), changed_capability.digest());
        assert_eq!(original.digest(), changed_example.digest());
    }

    fn fixture() -> JsonDocument {
        document(include_bytes!("../fixtures/a2a-1.0-agent-card.json"))
    }

    fn document(body: &[u8]) -> JsonDocument {
        JsonDocument::new(
            body.to_vec(),
            DurationMillis::new(60_000).expect("测试缓存时限有效"),
        )
    }

    fn source_url() -> AgentCardSourceUrl {
        AgentCardSourceUrl::new("https://agent.example/.well-known/agent-card.json".to_owned())
            .expect("测试来源有效")
    }
}
