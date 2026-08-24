use std::collections::BTreeSet;

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{AgentCardSnapshotRepository, PortFuture},
};
use agent_room_domain::{
    agent_cards::{
        AgentCardCapabilities, AgentCardDigest, AgentCardEndpoint, AgentCardExtension,
        AgentCardProtocolVersion, AgentCardProvider, AgentCardSecurityScheme,
        AgentCardSecuritySchemeKind, AgentCardSkill, AgentCardSnapshot, AgentCardSnapshotFields,
        AgentCardSourceUrl, AgentCardTransport, AgentCardVerificationState,
        AgentEndpointVerificationState, NormalizedAgentCard, NormalizedAgentCardFields,
    },
    ids::{AgentCardSnapshotId, AgentId},
    time::UtcMillis,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{PostgresRepositories, agents::decode_column, error::map_sqlx_error};

const STORED_CARD_SCHEMA_VERSION: u16 = 1;
const MAXIMUM_RETAINED_SNAPSHOTS: i64 = 10;
const RETENTION_DAYS: i32 = 90;

impl AgentCardSnapshotRepository for PostgresRepositories {
    fn find_latest(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentCardSnapshot>>> {
        Box::pin(async move {
            let operation = "agent_card_snapshot.find_latest";
            let row = sqlx::query(
                r"SELECT id, source_url, canonical_digest, normalized_card,
                         verification_state,
                         floor(extract(epoch FROM fetched_at) * 1000)::bigint AS fetched_at_ms,
                         floor(extract(epoch FROM expires_at) * 1000)::bigint AS expires_at_ms
                  FROM agent_room.agent_card_snapshot
                  WHERE agent_id = $1
                  ORDER BY fetched_at DESC, id DESC
                  LIMIT 1",
            )
            .bind(agent_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;

            row.map(|row| decode_snapshot(&row, agent_id, operation))
                .transpose()
        })
    }

    fn save<'a>(
        &'a self,
        snapshot: &'a AgentCardSnapshot,
    ) -> PortFuture<'a, RepositoryResult<AgentCardSnapshot>> {
        Box::pin(async move {
            let operation = "agent_card_snapshot.save";
            let normalized_card = encode_card(snapshot.card(), operation)?;
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            let result = async {
                lock_snapshot_history(&mut transaction, snapshot.agent_id()).await?;
                insert_snapshot(&mut transaction, snapshot, normalized_card).await?;
                prune_snapshot_history(&mut transaction, snapshot.agent_id(), snapshot.fetched_at())
                    .await
            }
            .await;

            if let Err(error) = result {
                transaction.rollback().await.map_err(|rollback| {
                    map_sqlx_error("agent_card_snapshot.rollback", &rollback)
                })?;
                return Err(error);
            }
            transaction
                .commit()
                .await
                .map_err(|error| map_sqlx_error("agent_card_snapshot.commit", &error))?;
            Ok(snapshot.clone())
        })
    }
}

async fn lock_snapshot_history(
    transaction: &mut Transaction<'_, Postgres>,
    agent_id: AgentId,
) -> RepositoryResult<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(agent_id.to_string())
        .execute(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error("agent_card_snapshot.lock", &error))?;
    Ok(())
}

async fn insert_snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    snapshot: &AgentCardSnapshot,
    normalized_card: Value,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.agent_card_snapshot (
            id, agent_id, source_url, canonical_digest, normalized_card,
            verification_state, fetched_at, expires_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6,
            to_timestamp($7::double precision / 1000.0),
            to_timestamp($8::double precision / 1000.0)
        )",
    )
    .bind(snapshot.id().as_uuid())
    .bind(snapshot.agent_id().as_uuid())
    .bind(snapshot.source_url())
    .bind(snapshot.digest().as_bytes().as_slice())
    .bind(normalized_card)
    .bind(snapshot.stored_verification().as_str())
    .bind(snapshot.fetched_at().value())
    .bind(snapshot.expires_at().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("agent_card_snapshot.insert", &error))?;
    Ok(())
}

async fn prune_snapshot_history(
    transaction: &mut Transaction<'_, Postgres>,
    agent_id: AgentId,
    reference_time: UtcMillis,
) -> RepositoryResult<()> {
    sqlx::query(
        r"DELETE FROM agent_room.agent_card_snapshot AS snapshot
          WHERE snapshot.agent_id = $1
            AND (
              snapshot.fetched_at <
                to_timestamp($2::double precision / 1000.0) - make_interval(days => $3::integer)
              OR snapshot.id NOT IN (
                SELECT retained.id
                FROM agent_room.agent_card_snapshot AS retained
                WHERE retained.agent_id = $1
                ORDER BY retained.fetched_at DESC, retained.id DESC
                LIMIT $4
              )
            )",
    )
    .bind(agent_id.as_uuid())
    .bind(reference_time.value())
    .bind(RETENTION_DAYS)
    .bind(MAXIMUM_RETAINED_SNAPSHOTS)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error("agent_card_snapshot.prune", &error))?;
    Ok(())
}

fn decode_snapshot(
    row: &PgRow,
    agent_id: AgentId,
    operation: &'static str,
) -> RepositoryResult<AgentCardSnapshot> {
    let id: uuid::Uuid = decode_column(row, "id", operation)?;
    let source_url: String = decode_column(row, "source_url", operation)?;
    let digest: Vec<u8> = decode_column(row, "canonical_digest", operation)?;
    let card: Value = decode_column(row, "normalized_card", operation)?;
    let verification: String = decode_column(row, "verification_state", operation)?;
    let fetched_at: i64 = decode_column(row, "fetched_at_ms", operation)?;
    let expires_at: Option<i64> = decode_column(row, "expires_at_ms", operation)?;
    AgentCardSnapshot::new(AgentCardSnapshotFields {
        id: AgentCardSnapshotId::from_uuid(id),
        agent_id,
        source_url: AgentCardSourceUrl::new(source_url).map_err(|_| corrupt_data(operation))?,
        digest: AgentCardDigest::new(digest).map_err(|_| corrupt_data(operation))?,
        card: decode_card(card, operation)?,
        verification: AgentCardVerificationState::try_from(verification.as_str())
            .map_err(|_| corrupt_data(operation))?,
        fetched_at: UtcMillis::new(fetched_at).map_err(|_| corrupt_data(operation))?,
        expires_at: UtcMillis::new(expires_at.ok_or_else(|| corrupt_data(operation))?)
            .map_err(|_| corrupt_data(operation))?,
    })
    .map_err(|_| corrupt_data(operation))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredNormalizedAgentCard {
    schema_version: u16,
    name: String,
    description: String,
    provider: Option<StoredProvider>,
    version: String,
    endpoints: Vec<StoredEndpoint>,
    capabilities: StoredCapabilities,
    security_schemes: Vec<StoredSecurityScheme>,
    default_input_modes: Vec<String>,
    default_output_modes: Vec<String>,
    skills: Vec<StoredSkill>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredProvider {
    organization: String,
    url: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredEndpoint {
    url: String,
    transport: String,
    protocol_version: String,
    tenant: Option<String>,
    verification: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredCapabilities {
    streaming: bool,
    push_notifications: bool,
    extended_agent_card: bool,
    extensions: Vec<StoredExtension>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredExtension {
    uri: String,
    description: String,
    required: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredSecurityScheme {
    name: String,
    kind: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredSkill {
    id: String,
    name: String,
    description: String,
    tags: Vec<String>,
    input_modes: Vec<String>,
    output_modes: Vec<String>,
}

impl From<&NormalizedAgentCard> for StoredNormalizedAgentCard {
    fn from(card: &NormalizedAgentCard) -> Self {
        Self {
            schema_version: STORED_CARD_SCHEMA_VERSION,
            name: card.name().to_owned(),
            description: card.description().to_owned(),
            provider: card.provider().map(|provider| StoredProvider {
                organization: provider.organization().to_owned(),
                url: provider.url().to_owned(),
            }),
            version: card.version().to_owned(),
            endpoints: card
                .endpoints()
                .iter()
                .map(|endpoint| StoredEndpoint {
                    url: endpoint.url().to_owned(),
                    transport: endpoint.transport().as_str().to_owned(),
                    protocol_version: format!(
                        "{}.{}",
                        endpoint.protocol_version().major(),
                        endpoint.protocol_version().minor()
                    ),
                    tenant: endpoint.tenant().map(str::to_owned),
                    verification: endpoint.verification().as_str().to_owned(),
                })
                .collect(),
            capabilities: StoredCapabilities {
                streaming: card.capabilities().streaming(),
                push_notifications: card.capabilities().push_notifications(),
                extended_agent_card: card.capabilities().extended_agent_card(),
                extensions: card
                    .capabilities()
                    .extensions()
                    .iter()
                    .map(|extension| StoredExtension {
                        uri: extension.uri().to_owned(),
                        description: extension.description().to_owned(),
                        required: extension.required(),
                    })
                    .collect(),
            },
            security_schemes: card
                .security_schemes()
                .iter()
                .map(|scheme| StoredSecurityScheme {
                    name: scheme.name().to_owned(),
                    kind: scheme.kind().as_str().to_owned(),
                })
                .collect(),
            default_input_modes: card.default_input_modes().to_vec(),
            default_output_modes: card.default_output_modes().to_vec(),
            skills: card
                .skills()
                .iter()
                .map(|skill| StoredSkill {
                    id: skill.id().to_owned(),
                    name: skill.name().to_owned(),
                    description: skill.description().to_owned(),
                    tags: skill.tags().to_vec(),
                    input_modes: skill.input_modes().to_vec(),
                    output_modes: skill.output_modes().to_vec(),
                })
                .collect(),
        }
    }
}

fn encode_card(card: &NormalizedAgentCard, operation: &'static str) -> RepositoryResult<Value> {
    serde_json::to_value(StoredNormalizedAgentCard::from(card)).map_err(|_| corrupt_data(operation))
}

fn decode_card(value: Value, operation: &'static str) -> RepositoryResult<NormalizedAgentCard> {
    let stored = serde_json::from_value::<StoredNormalizedAgentCard>(value)
        .map_err(|_| corrupt_data(operation))?;
    restore_card(stored).map_err(|()| corrupt_data(operation))
}

fn restore_card(stored: StoredNormalizedAgentCard) -> Result<NormalizedAgentCard, ()> {
    if stored.schema_version != STORED_CARD_SCHEMA_VERSION {
        return Err(());
    }
    let endpoints = stored
        .endpoints
        .into_iter()
        .map(|endpoint| {
            AgentCardEndpoint::new(
                endpoint.url,
                AgentCardTransport::try_from(endpoint.transport.as_str()).map_err(|_| ())?,
                AgentCardProtocolVersion::parse(&endpoint.protocol_version).map_err(|_| ())?,
                endpoint.tenant,
                endpoint_verification(&endpoint.verification)?,
            )
            .map_err(|_| ())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let extensions = stored
        .capabilities
        .extensions
        .into_iter()
        .map(|extension| {
            AgentCardExtension::new(extension.uri, extension.description, extension.required)
                .map_err(|_| ())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let supported_extensions = extensions
        .iter()
        .map(|extension| extension.uri().to_owned())
        .collect::<BTreeSet<_>>();
    let capabilities = AgentCardCapabilities::new(
        stored.capabilities.streaming,
        stored.capabilities.push_notifications,
        stored.capabilities.extended_agent_card,
        extensions,
        &supported_extensions,
    )
    .map_err(|_| ())?;
    let security_schemes = stored
        .security_schemes
        .into_iter()
        .map(|scheme| {
            AgentCardSecurityScheme::new(scheme.name, security_scheme_kind(&scheme.kind)?)
                .map_err(|_| ())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let skills = stored
        .skills
        .into_iter()
        .map(|skill| {
            AgentCardSkill::new(
                skill.id,
                skill.name,
                skill.description,
                skill.tags,
                skill.input_modes,
                skill.output_modes,
            )
            .map_err(|_| ())
        })
        .collect::<Result<Vec<_>, _>>()?;
    NormalizedAgentCard::new(NormalizedAgentCardFields {
        name: stored.name,
        description: stored.description,
        provider: stored
            .provider
            .map(|provider| AgentCardProvider::new(provider.organization, provider.url))
            .transpose()
            .map_err(|_| ())?,
        version: stored.version,
        endpoints,
        capabilities,
        security_schemes,
        default_input_modes: stored.default_input_modes,
        default_output_modes: stored.default_output_modes,
        skills,
    })
    .map_err(|_| ())
}

fn endpoint_verification(value: &str) -> Result<AgentEndpointVerificationState, ()> {
    match value {
        "verified" => Ok(AgentEndpointVerificationState::Verified),
        "declared" => Ok(AgentEndpointVerificationState::Declared),
        _ => Err(()),
    }
}

fn security_scheme_kind(value: &str) -> Result<AgentCardSecuritySchemeKind, ()> {
    match value {
        "api_key" => Ok(AgentCardSecuritySchemeKind::ApiKey),
        "http" => Ok(AgentCardSecuritySchemeKind::Http),
        "oauth2" => Ok(AgentCardSecuritySchemeKind::OAuth2),
        "open_id_connect" => Ok(AgentCardSecuritySchemeKind::OpenIdConnect),
        "mutual_tls" => Ok(AgentCardSecuritySchemeKind::MutualTls),
        _ => Err(()),
    }
}

const fn corrupt_data(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::CorruptData)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use agent_room_domain::agent_cards::{
        AgentCardCapabilities, AgentCardEndpoint, AgentCardProtocolVersion,
        AgentCardSecurityScheme, AgentCardSecuritySchemeKind, AgentCardSkill, AgentCardTransport,
        AgentEndpointVerificationState, NormalizedAgentCard, NormalizedAgentCardFields,
    };
    use serde_json::json;

    use super::{decode_card, encode_card};

    #[test]
    fn 规范化资料可无损往返且不写入原始敏感字段() {
        let card = fixture_card();
        let encoded = encode_card(&card, "test.encode").expect("资料应可编码");
        let decoded = decode_card(encoded.clone(), "test.decode").expect("资料应可解码");

        assert_eq!(decoded, card);
        assert_eq!(encoded["schemaVersion"], 1);
        assert!(encoded.get("signatures").is_none());
        assert!(encoded.get("examples").is_none());
    }

    #[test]
    fn 未知持久化版本会响亮失败() {
        let mut encoded = encode_card(&fixture_card(), "test.encode").expect("资料应可编码");
        encoded["schemaVersion"] = json!(2);

        assert!(decode_card(encoded, "test.decode").is_err());
    }

    fn fixture_card() -> NormalizedAgentCard {
        let capabilities =
            AgentCardCapabilities::new(true, false, false, Vec::new(), &BTreeSet::new())
                .expect("测试能力有效");
        NormalizedAgentCard::new(NormalizedAgentCardFields {
            name: "测试 Agent".to_owned(),
            description: "仅包含公开字段".to_owned(),
            provider: None,
            version: "1.0.0".to_owned(),
            endpoints: vec![
                AgentCardEndpoint::new(
                    "https://agent.example/a2a".to_owned(),
                    AgentCardTransport::JsonRpc,
                    AgentCardProtocolVersion::V1_0,
                    None,
                    AgentEndpointVerificationState::Verified,
                )
                .expect("测试端点有效"),
            ],
            capabilities,
            security_schemes: vec![
                AgentCardSecurityScheme::new(
                    "oauth".to_owned(),
                    AgentCardSecuritySchemeKind::OAuth2,
                )
                .expect("测试认证摘要有效"),
            ],
            default_input_modes: vec!["text/plain".to_owned()],
            default_output_modes: vec!["text/plain".to_owned()],
            skills: vec![
                AgentCardSkill::new(
                    "chat".to_owned(),
                    "聊天".to_owned(),
                    "公开聊天能力".to_owned(),
                    vec!["chat".to_owned()],
                    Vec::new(),
                    Vec::new(),
                )
                .expect("测试技能有效"),
            ],
        })
        .expect("测试资料有效")
    }
}
