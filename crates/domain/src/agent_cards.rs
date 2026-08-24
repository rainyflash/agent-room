use std::collections::BTreeSet;

use url::Url;

use crate::{
    DomainError, DomainResult,
    ids::{AgentCardSnapshotId, AgentId},
    time::UtcMillis,
};

const MAX_NAME_LENGTH: usize = 128;
const MAX_DESCRIPTION_LENGTH: usize = 2_048;
const MAX_VERSION_LENGTH: usize = 64;
const MAX_PROVIDER_NAME_LENGTH: usize = 128;
const MAX_ENDPOINTS: usize = 8;
const MAX_TENANT_LENGTH: usize = 256;
const MAX_SKILLS: usize = 64;
const MAX_SKILL_ID_LENGTH: usize = 128;
const MAX_SKILL_NAME_LENGTH: usize = 128;
const MAX_SKILL_DESCRIPTION_LENGTH: usize = 2_048;
const MAX_TAGS_PER_SKILL: usize = 32;
const MAX_TAG_LENGTH: usize = 64;
const MAX_MEDIA_MODES: usize = 32;
const MAX_MEDIA_MODE_LENGTH: usize = 128;
const MAX_EXTENSIONS: usize = 32;
const MAX_EXTENSION_DESCRIPTION_LENGTH: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCardVerificationState {
    Verified,
    Unverified,
    Invalid,
    Expired,
}

impl AgentCardVerificationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Unverified => "unverified",
            Self::Invalid => "invalid",
            Self::Expired => "expired",
        }
    }
}

impl TryFrom<&str> for AgentCardVerificationState {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "verified" => Ok(Self::Verified),
            "unverified" => Ok(Self::Unverified),
            "invalid" => Ok(Self::Invalid),
            "expired" => Ok(Self::Expired),
            _ => Err(DomainError::Validation {
                field: "agent_card_verification_state",
                reason: "包含未知状态",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentEndpointVerificationState {
    Verified,
    Declared,
}

impl AgentEndpointVerificationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Declared => "declared",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCardTransport {
    JsonRpc,
    Grpc,
    HttpJson,
}

impl AgentCardTransport {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::JsonRpc => "JSONRPC",
            Self::Grpc => "GRPC",
            Self::HttpJson => "HTTP+JSON",
        }
    }
}

impl TryFrom<&str> for AgentCardTransport {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "JSONRPC" => Ok(Self::JsonRpc),
            "GRPC" => Ok(Self::Grpc),
            "HTTP+JSON" => Ok(Self::HttpJson),
            _ => Err(DomainError::Validation {
                field: "agent_card_protocol_binding",
                reason: "当前版本不支持该协议绑定",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AgentCardProtocolVersion {
    major: u16,
    minor: u16,
}

impl AgentCardProtocolVersion {
    pub const V1_0: Self = Self { major: 1, minor: 0 };

    /// 解析 A2A `major.minor` 协议版本。
    ///
    /// # Errors
    ///
    /// 版本格式错误、包含多余段或数值溢出时失败。
    pub fn parse(value: &str) -> DomainResult<Self> {
        let mut parts = value.split('.');
        let major = parts
            .next()
            .and_then(|part| part.parse::<u16>().ok())
            .ok_or(DomainError::Validation {
                field: "agent_card_protocol_version",
                reason: "必须使用 major.minor 格式",
            })?;
        let minor = parts
            .next()
            .and_then(|part| part.parse::<u16>().ok())
            .ok_or(DomainError::Validation {
                field: "agent_card_protocol_version",
                reason: "必须使用 major.minor 格式",
            })?;
        if parts.next().is_some() {
            return Err(DomainError::Validation {
                field: "agent_card_protocol_version",
                reason: "必须使用 major.minor 格式",
            });
        }
        Ok(Self { major, minor })
    }

    pub const fn major(self) -> u16 {
        self.major
    }

    pub const fn minor(self) -> u16 {
        self.minor
    }

    pub const fn is_supported(self) -> bool {
        self.major == Self::V1_0.major && self.minor == Self::V1_0.minor
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCardEndpoint {
    url: String,
    transport: AgentCardTransport,
    protocol_version: AgentCardProtocolVersion,
    tenant: Option<String>,
    verification: AgentEndpointVerificationState,
}

impl AgentCardEndpoint {
    /// 创建已规范化的 A2A 服务端点。
    ///
    /// # Errors
    ///
    /// URL 不是无凭据 HTTPS 地址、版本不受支持或租户值无效时失败。
    pub fn new(
        url: String,
        transport: AgentCardTransport,
        protocol_version: AgentCardProtocolVersion,
        tenant: Option<String>,
        verification: AgentEndpointVerificationState,
    ) -> DomainResult<Self> {
        validate_https_url("agent_card_endpoint_url", &url)?;
        if !protocol_version.is_supported() {
            return Err(DomainError::Validation {
                field: "agent_card_protocol_version",
                reason: "当前实现不支持该协议版本",
            });
        }
        if let Some(value) = tenant.as_deref() {
            validate_text("agent_card_tenant", value, MAX_TENANT_LENGTH, false)?;
        }
        Ok(Self {
            url,
            transport,
            protocol_version,
            tenant,
            verification,
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub const fn transport(&self) -> AgentCardTransport {
        self.transport
    }

    pub const fn protocol_version(&self) -> AgentCardProtocolVersion {
        self.protocol_version
    }

    pub fn tenant(&self) -> Option<&str> {
        self.tenant.as_deref()
    }

    pub const fn verification(&self) -> AgentEndpointVerificationState {
        self.verification
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCardProvider {
    organization: String,
    url: String,
}

impl AgentCardProvider {
    /// 创建公开提供方资料。
    ///
    /// # Errors
    ///
    /// 组织名无效或 URL 不是无凭据 HTTPS 地址时失败。
    pub fn new(organization: String, url: String) -> DomainResult<Self> {
        validate_text(
            "agent_card_provider_organization",
            &organization,
            MAX_PROVIDER_NAME_LENGTH,
            false,
        )?;
        validate_https_url("agent_card_provider_url", &url)?;
        Ok(Self { organization, url })
    }

    pub fn organization(&self) -> &str {
        &self.organization
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCardExtension {
    uri: String,
    description: String,
    required: bool,
}

impl AgentCardExtension {
    /// 创建只保留公开声明字段的扩展。
    ///
    /// # Errors
    ///
    /// 扩展 URI 或说明无效时失败。
    pub fn new(uri: String, description: String, required: bool) -> DomainResult<Self> {
        let parsed = Url::parse(&uri).map_err(|_| DomainError::Validation {
            field: "agent_card_extension_uri",
            reason: "必须是绝对 URI",
        })?;
        if parsed.cannot_be_a_base() && parsed.scheme() != "urn" {
            return Err(DomainError::Validation {
                field: "agent_card_extension_uri",
                reason: "仅接受 HTTPS URL 或 URN",
            });
        }
        if parsed.scheme() != "https" && parsed.scheme() != "urn" {
            return Err(DomainError::Validation {
                field: "agent_card_extension_uri",
                reason: "仅接受 HTTPS URL 或 URN",
            });
        }
        validate_text(
            "agent_card_extension_description",
            &description,
            MAX_EXTENSION_DESCRIPTION_LENGTH,
            true,
        )?;
        Ok(Self {
            uri,
            description,
            required,
        })
    }

    pub fn uri(&self) -> &str {
        &self.uri
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub const fn required(&self) -> bool {
        self.required
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCardCapabilities {
    streaming: bool,
    push_notifications: bool,
    extended_agent_card: bool,
    extensions: Vec<AgentCardExtension>,
}

impl AgentCardCapabilities {
    /// 创建有界能力集合。
    ///
    /// # Errors
    ///
    /// 扩展数量过多、URI 重复或存在当前实现不理解的必需扩展时失败。
    pub fn new(
        streaming: bool,
        push_notifications: bool,
        extended_agent_card: bool,
        extensions: Vec<AgentCardExtension>,
        supported_extension_uris: &BTreeSet<String>,
    ) -> DomainResult<Self> {
        ensure_maximum("agent_card_extensions", extensions.len(), MAX_EXTENSIONS)?;
        ensure_unique(
            "agent_card_extension_uri",
            extensions.iter().map(AgentCardExtension::uri),
        )?;
        if extensions.iter().any(|extension| {
            extension.required() && !supported_extension_uris.contains(extension.uri())
        }) {
            return Err(DomainError::Validation {
                field: "agent_card_extensions",
                reason: "包含当前实现不支持的必需扩展",
            });
        }
        Ok(Self {
            streaming,
            push_notifications,
            extended_agent_card,
            extensions,
        })
    }

    pub const fn streaming(&self) -> bool {
        self.streaming
    }

    pub const fn push_notifications(&self) -> bool {
        self.push_notifications
    }

    pub const fn extended_agent_card(&self) -> bool {
        self.extended_agent_card
    }

    pub fn extensions(&self) -> &[AgentCardExtension] {
        &self.extensions
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCardSkill {
    id: String,
    name: String,
    description: String,
    tags: Vec<String>,
    input_modes: Vec<String>,
    output_modes: Vec<String>,
}

impl AgentCardSkill {
    /// 创建不含示例提示词和私有扩展的安全技能资料。
    ///
    /// # Errors
    ///
    /// 必填文本、标签或媒体类型超出边界时失败。
    pub fn new(
        id: String,
        name: String,
        description: String,
        tags: Vec<String>,
        input_modes: Vec<String>,
        output_modes: Vec<String>,
    ) -> DomainResult<Self> {
        validate_text("agent_card_skill_id", &id, MAX_SKILL_ID_LENGTH, false)?;
        validate_text("agent_card_skill_name", &name, MAX_SKILL_NAME_LENGTH, false)?;
        validate_text(
            "agent_card_skill_description",
            &description,
            MAX_SKILL_DESCRIPTION_LENGTH,
            true,
        )?;
        validate_string_set(
            "agent_card_skill_tags",
            &tags,
            MAX_TAGS_PER_SKILL,
            MAX_TAG_LENGTH,
            false,
        )?;
        validate_media_modes("agent_card_skill_input_modes", &input_modes)?;
        validate_media_modes("agent_card_skill_output_modes", &output_modes)?;
        Ok(Self {
            id,
            name,
            description,
            tags,
            input_modes,
            output_modes,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    pub fn input_modes(&self) -> &[String] {
        &self.input_modes
    }

    pub fn output_modes(&self) -> &[String] {
        &self.output_modes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedAgentCard {
    name: String,
    description: String,
    provider: Option<AgentCardProvider>,
    version: String,
    endpoints: Vec<AgentCardEndpoint>,
    capabilities: AgentCardCapabilities,
    default_input_modes: Vec<String>,
    default_output_modes: Vec<String>,
    skills: Vec<AgentCardSkill>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedAgentCardFields {
    pub name: String,
    pub description: String,
    pub provider: Option<AgentCardProvider>,
    pub version: String,
    pub endpoints: Vec<AgentCardEndpoint>,
    pub capabilities: AgentCardCapabilities,
    pub default_input_modes: Vec<String>,
    pub default_output_modes: Vec<String>,
    pub skills: Vec<AgentCardSkill>,
}

impl NormalizedAgentCard {
    /// 创建 A2A 1.0 安全字段的规范化快照。
    ///
    /// # Errors
    ///
    /// 必填字段、端点、媒体类型或技能集合无效时失败。
    pub fn new(fields: NormalizedAgentCardFields) -> DomainResult<Self> {
        validate_text("agent_card_name", &fields.name, MAX_NAME_LENGTH, false)?;
        validate_text(
            "agent_card_description",
            &fields.description,
            MAX_DESCRIPTION_LENGTH,
            true,
        )?;
        validate_text(
            "agent_card_version",
            &fields.version,
            MAX_VERSION_LENGTH,
            false,
        )?;
        ensure_maximum(
            "agent_card_endpoints",
            fields.endpoints.len(),
            MAX_ENDPOINTS,
        )?;
        if fields.endpoints.is_empty() {
            return Err(DomainError::Validation {
                field: "agent_card_endpoints",
                reason: "至少需要一个兼容端点",
            });
        }
        ensure_unique(
            "agent_card_endpoint",
            fields.endpoints.iter().map(AgentCardEndpoint::url),
        )?;
        validate_media_modes(
            "agent_card_default_input_modes",
            &fields.default_input_modes,
        )?;
        validate_media_modes(
            "agent_card_default_output_modes",
            &fields.default_output_modes,
        )?;
        ensure_maximum("agent_card_skills", fields.skills.len(), MAX_SKILLS)?;
        ensure_unique(
            "agent_card_skill_id",
            fields.skills.iter().map(AgentCardSkill::id),
        )?;
        Ok(Self {
            name: fields.name,
            description: fields.description,
            provider: fields.provider,
            version: fields.version,
            endpoints: fields.endpoints,
            capabilities: fields.capabilities,
            default_input_modes: fields.default_input_modes,
            default_output_modes: fields.default_output_modes,
            skills: fields.skills,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub const fn provider(&self) -> Option<&AgentCardProvider> {
        self.provider.as_ref()
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn endpoints(&self) -> &[AgentCardEndpoint] {
        &self.endpoints
    }

    pub const fn capabilities(&self) -> &AgentCardCapabilities {
        &self.capabilities
    }

    pub fn default_input_modes(&self) -> &[String] {
        &self.default_input_modes
    }

    pub fn default_output_modes(&self) -> &[String] {
        &self.default_output_modes
    }

    pub fn skills(&self) -> &[AgentCardSkill] {
        &self.skills
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCardDigest([u8; 32]);

impl AgentCardDigest {
    pub const fn from_array(value: [u8; 32]) -> Self {
        Self(value)
    }

    /// 从持久化字节恢复摘要。
    ///
    /// # Errors
    ///
    /// 摘要不是 32 字节时失败。
    pub fn new(value: Vec<u8>) -> DomainResult<Self> {
        let value = <[u8; 32]>::try_from(value).map_err(|_| DomainError::Validation {
            field: "agent_card_digest",
            reason: "必须是 32 字节 SHA-256 摘要",
        })?;
        Ok(Self(value))
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCardSourceUrl(String);

impl AgentCardSourceUrl {
    /// 创建 Agent Card 来源地址。
    ///
    /// # Errors
    ///
    /// 地址不是无凭据、无片段的绝对 HTTPS URL 时失败。
    pub fn new(value: String) -> DomainResult<Self> {
        validate_https_url("agent_card_source_url", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCardSnapshot {
    id: AgentCardSnapshotId,
    agent_id: AgentId,
    source_url: AgentCardSourceUrl,
    digest: AgentCardDigest,
    card: NormalizedAgentCard,
    verification: AgentCardVerificationState,
    fetched_at: UtcMillis,
    expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCardSnapshotFields {
    pub id: AgentCardSnapshotId,
    pub agent_id: AgentId,
    pub source_url: AgentCardSourceUrl,
    pub digest: AgentCardDigest,
    pub card: NormalizedAgentCard,
    pub verification: AgentCardVerificationState,
    pub fetched_at: UtcMillis,
    pub expires_at: UtcMillis,
}

impl AgentCardSnapshot {
    /// 创建不可变 Agent Card 快照。
    ///
    /// # Errors
    ///
    /// 来源不是安全 HTTPS URL，或过期时间不晚于抓取时间时失败。
    pub fn new(fields: AgentCardSnapshotFields) -> DomainResult<Self> {
        if fields.expires_at <= fields.fetched_at {
            return Err(DomainError::Validation {
                field: "agent_card_expires_at",
                reason: "必须晚于抓取时间",
            });
        }
        Ok(Self {
            id: fields.id,
            agent_id: fields.agent_id,
            source_url: fields.source_url,
            digest: fields.digest,
            card: fields.card,
            verification: fields.verification,
            fetched_at: fields.fetched_at,
            expires_at: fields.expires_at,
        })
    }

    pub const fn id(&self) -> AgentCardSnapshotId {
        self.id
    }

    pub const fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    pub fn source_url(&self) -> &str {
        self.source_url.as_str()
    }

    pub const fn digest(&self) -> &AgentCardDigest {
        &self.digest
    }

    pub const fn card(&self) -> &NormalizedAgentCard {
        &self.card
    }

    pub const fn stored_verification(&self) -> AgentCardVerificationState {
        self.verification
    }

    pub const fn fetched_at(&self) -> UtcMillis {
        self.fetched_at
    }

    pub const fn expires_at(&self) -> UtcMillis {
        self.expires_at
    }

    pub fn verification_at(&self, now: UtcMillis) -> AgentCardVerificationState {
        if now >= self.expires_at {
            AgentCardVerificationState::Expired
        } else {
            self.verification
        }
    }
}

fn validate_https_url(field: &'static str, value: &str) -> DomainResult<()> {
    let parsed = Url::parse(value).map_err(|_| DomainError::Validation {
        field,
        reason: "必须是绝对 HTTPS URL",
    })?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(DomainError::Validation {
            field,
            reason: "必须是无凭据、无片段的绝对 HTTPS URL",
        });
    }
    Ok(())
}

fn validate_media_modes(field: &'static str, values: &[String]) -> DomainResult<()> {
    validate_string_set(field, values, MAX_MEDIA_MODES, MAX_MEDIA_MODE_LENGTH, false)
}

fn validate_string_set(
    field: &'static str,
    values: &[String],
    maximum_items: usize,
    maximum_length: usize,
    allow_empty: bool,
) -> DomainResult<()> {
    ensure_maximum(field, values.len(), maximum_items)?;
    for value in values {
        validate_text(field, value, maximum_length, allow_empty)?;
    }
    ensure_unique(field, values.iter().map(String::as_str))
}

fn validate_text(
    field: &'static str,
    value: &str,
    maximum_length: usize,
    allow_empty: bool,
) -> DomainResult<()> {
    let length = value.chars().count();
    if (!allow_empty && value.trim().is_empty())
        || length > maximum_length
        || value.chars().any(char::is_control)
    {
        return Err(DomainError::Validation {
            field,
            reason: "为空、超长或包含控制字符",
        });
    }
    Ok(())
}

fn ensure_maximum(field: &'static str, actual: usize, maximum: usize) -> DomainResult<()> {
    if actual > maximum {
        Err(DomainError::Validation {
            field,
            reason: "超过允许的元素数量",
        })
    } else {
        Ok(())
    }
}

fn ensure_unique<'a>(
    field: &'static str,
    values: impl IntoIterator<Item = &'a str>,
) -> DomainResult<()> {
    let mut unique = BTreeSet::new();
    if values.into_iter().any(|value| !unique.insert(value)) {
        Err(DomainError::Validation {
            field,
            reason: "包含重复值",
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use uuid::Uuid;

    use super::{
        AgentCardCapabilities, AgentCardDigest, AgentCardEndpoint, AgentCardProtocolVersion,
        AgentCardSkill, AgentCardSnapshot, AgentCardSnapshotFields, AgentCardSourceUrl,
        AgentCardTransport, AgentCardVerificationState, AgentEndpointVerificationState,
        NormalizedAgentCard, NormalizedAgentCardFields,
    };
    use crate::{
        ids::{AgentCardSnapshotId, AgentId},
        time::UtcMillis,
    };

    #[test]
    fn 接受_a2a_1_0_安全字段并按时间判定过期() {
        let card = card("1.0", "研究助手");
        let snapshot = AgentCardSnapshot::new(AgentCardSnapshotFields {
            id: AgentCardSnapshotId::from_uuid(Uuid::now_v7()),
            agent_id: AgentId::from_uuid(Uuid::now_v7()),
            source_url: AgentCardSourceUrl::new(
                "https://agent.example/.well-known/agent-card.json".to_owned(),
            )
            .expect("测试来源有效"),
            digest: AgentCardDigest::from_array([7; 32]),
            card,
            verification: AgentCardVerificationState::Verified,
            fetched_at: time(1_000),
            expires_at: time(2_000),
        })
        .expect("合法 Card 快照可创建");

        assert_eq!(
            snapshot.verification_at(time(1_999)),
            AgentCardVerificationState::Verified
        );
        assert_eq!(
            snapshot.verification_at(time(2_000)),
            AgentCardVerificationState::Expired
        );
    }

    #[test]
    fn 拒绝不支持版本和带凭据端点() {
        assert!(AgentCardProtocolVersion::parse("1.0.0").is_err());
        let unsupported = AgentCardProtocolVersion::parse("2.0").expect("版本格式本身有效");
        assert!(
            AgentCardEndpoint::new(
                "https://agent.example/a2a".to_owned(),
                AgentCardTransport::JsonRpc,
                unsupported,
                None,
                AgentEndpointVerificationState::Declared,
            )
            .is_err()
        );
        assert!(
            AgentCardEndpoint::new(
                "https://user:secret@agent.example/a2a".to_owned(),
                AgentCardTransport::JsonRpc,
                AgentCardProtocolVersion::V1_0,
                None,
                AgentEndpointVerificationState::Declared,
            )
            .is_err()
        );
    }

    #[test]
    fn 拒绝重复技能与空兼容端点() {
        let skill = AgentCardSkill::new(
            "research".to_owned(),
            "研究".to_owned(),
            String::new(),
            vec!["research".to_owned()],
            vec!["text/plain".to_owned()],
            vec!["text/plain".to_owned()],
        )
        .expect("技能有效");
        let result = NormalizedAgentCard::new(NormalizedAgentCardFields {
            name: "研究助手".to_owned(),
            description: String::new(),
            provider: None,
            version: "1.0.0".to_owned(),
            endpoints: Vec::new(),
            capabilities: AgentCardCapabilities::new(
                false,
                false,
                false,
                Vec::new(),
                &BTreeSet::new(),
            )
            .expect("空能力有效"),
            default_input_modes: vec!["text/plain".to_owned()],
            default_output_modes: vec!["text/plain".to_owned()],
            skills: vec![skill.clone(), skill],
        });

        assert!(result.is_err());
    }

    fn card(protocol_version: &str, name: &str) -> NormalizedAgentCard {
        let endpoint = AgentCardEndpoint::new(
            "https://agent.example/a2a".to_owned(),
            AgentCardTransport::HttpJson,
            AgentCardProtocolVersion::parse(protocol_version).expect("测试协议版本有效"),
            None,
            AgentEndpointVerificationState::Verified,
        )
        .expect("测试端点有效");
        NormalizedAgentCard::new(NormalizedAgentCardFields {
            name: name.to_owned(),
            description: "公开能力资料".to_owned(),
            provider: None,
            version: "1.2.0".to_owned(),
            endpoints: vec![endpoint],
            capabilities: AgentCardCapabilities::new(
                true,
                false,
                false,
                Vec::new(),
                &BTreeSet::new(),
            )
            .expect("测试能力有效"),
            default_input_modes: vec!["text/plain".to_owned()],
            default_output_modes: vec!["text/plain".to_owned()],
            skills: Vec::new(),
        })
        .expect("测试 Card 有效")
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
