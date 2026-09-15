use crate::{PostgresRepositories, agents::decode_column, error::map_sqlx_error, transaction};
use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{AutomationScopeAuthority, AutomationSendAuthorityRequest, MatrixRoomId, PortFuture},
    reception::{
        ReceptionCommand, ReceptionProgress, ReceptionRecord, ReceptionRepository,
        ReceptionRequest, ReceptionStatus,
    },
};
use agent_room_domain::{
    ids::{AgentId, DeviceId, PrincipalId, RoomCatalogId},
    reception::{ReceptionExecution, ReceptionMode, ReceptionOwner, ReceptionTransition},
};
use sqlx::{Postgres, Transaction, postgres::PgRow};
use uuid::Uuid;

const OP: &str = "reception.execute";
fn failure(kind: RepositoryErrorKind) -> RepositoryError {
    RepositoryError::new(OP, kind)
}

impl ReceptionRepository for PostgresRepositories {
    fn execute<'a>(
        &'a self,
        principal: PrincipalId,
        device: DeviceId,
        request: &'a ReceptionRequest,
    ) -> PortFuture<'a, RepositoryResult<ReceptionRecord>> {
        Box::pin(async move {
            if !request.valid() {
                return Err(failure(RepositoryErrorKind::Constraint));
            }
            if !matches!(request.command, ReceptionCommand::Release) {
                let authority = AutomationSendAuthorityRequest {
                    principal_id: principal,
                    device_id: device,
                    agent_id: request.agent_id.into(),
                    agent_instance_id: request.instance_id.into(),
                    room_catalog_id: request.catalog_id.into(),
                    matrix_room_id: MatrixRoomId::new(request.room_id.clone())
                        .map_err(|_| failure(RepositoryErrorKind::Constraint))?,
                };
                if self.inspect_send(&authority).await?.is_none() {
                    return Err(failure(RepositoryErrorKind::Forbidden));
                }
            }
            let mut tx = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(OP, &error))?;
            let result = execute_locked(&mut tx, principal, device, request).await;
            transaction::finish(tx, result, OP).await
        })
    }
    fn list(
        &self,
        principal: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ReceptionRecord>>> {
        Box::pin(async move {
            sqlx::query("SELECT reception.*, device.label AS device_label FROM agent_room.reception_execution AS reception JOIN agent_room.device AS device ON device.id = reception.device_id WHERE EXISTS(SELECT 1 FROM agent_room.agent_ownership AS ownership JOIN agent_room.agent AS agent ON agent.id = ownership.agent_id WHERE ownership.agent_id = reception.agent_id AND ownership.principal_id = $1 AND ownership.role IN ('owner','operator') AND ownership.revoked_at IS NULL AND agent.lifecycle_state = 'active') ORDER BY reception.last_seen_unix_ms DESC LIMIT 128").bind(principal.as_uuid()).fetch_all(self.pool()).await.map_err(|error| map_sqlx_error(OP, &error))?.iter().map(decode).collect()
        })
    }
    fn drain(
        &self,
        principal: PrincipalId,
        agent: AgentId,
        catalog: RoomCatalogId,
        next_device: Option<DeviceId>,
    ) -> PortFuture<'_, RepositoryResult<ReceptionRecord>> {
        Box::pin(async move {
            let mut tx = self
                .pool()
                .begin()
                .await
                .map_err(|error| map_sqlx_error(OP, &error))?;
            let result = async {
                lock_owned_agent(&mut tx, principal, agent.as_uuid()).await?;
                if let Some(device) = next_device {
                    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_room.device WHERE id=$1 AND principal_id=$2 AND trust_state='verified' AND revoked_at IS NULL)")
                        .bind(device.as_uuid()).bind(principal.as_uuid()).fetch_one(&mut *tx).await.map_err(|error| map_sqlx_error(OP, &error))?;
                    if !allowed { return Err(failure(RepositoryErrorKind::Forbidden)); }
                }
                let record = find(&mut tx, agent.as_uuid(), catalog.as_uuid()).await?.ok_or_else(|| failure(RepositoryErrorKind::NotFound))?;
                let mut next = execution(&record).apply(ReceptionTransition::Drain(next_device)).map_err(|_| failure(RepositoryErrorKind::Conflict))?;
                // A crashed/offline executor can only be released once its actual Matrix
                // device was revoked. An expired presence lease alone is not evidence.
                let revoked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_room.agent_instance WHERE id=$1 AND revoked_at IS NOT NULL AND matrix_device_revoked_at IS NOT NULL)")
                    .bind(record.instance_id).fetch_one(&mut *tx).await.map_err(|error| map_sqlx_error(OP, &error))?;
                if revoked { next.mode = ReceptionMode::Idle; }
                sqlx::query("UPDATE agent_room.reception_execution SET state=$3, next_device_id=$4 WHERE agent_id=$1 AND catalog_id=$2")
                    .bind(agent.as_uuid()).bind(catalog.as_uuid()).bind(status_text(status(next.mode))).bind(next_device.map(DeviceId::as_uuid))
                    .execute(&mut *tx).await.map_err(|error| map_sqlx_error(OP, &error))?;
                find(&mut tx, agent.as_uuid(), catalog.as_uuid()).await?.ok_or_else(|| failure(RepositoryErrorKind::CorruptData))
            }.await;
            transaction::finish(tx, result, OP).await
        })
    }
}

async fn execute_locked(
    tx: &mut Transaction<'_, Postgres>,
    principal: PrincipalId,
    device: DeviceId,
    request: &ReceptionRequest,
) -> RepositoryResult<ReceptionRecord> {
    lock_owned_agent(tx, principal, request.agent_id).await?;
    let instance_allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_room.agent_instance WHERE id = $1 AND agent_id = $2 AND device_id = $3 AND revoked_at IS NULL)")
                    .bind(request.instance_id).bind(request.agent_id).bind(device.as_uuid()).fetch_one(&mut **tx).await.map_err(|error| map_sqlx_error(OP, &error))?;
    if !instance_allowed {
        return Err(failure(RepositoryErrorKind::Forbidden));
    }
    let existing = find(tx, request.agent_id, request.catalog_id).await?;
    let owner = ReceptionOwner {
        instance_id: request.instance_id.into(),
        device_id: device,
        run_id: request.run_id,
    };
    let Some(mut record) = existing else {
        return create(tx, device, request).await;
    };
    if record.room_id != request.room_id {
        return Err(failure(RepositoryErrorKind::Conflict));
    }
    let mut execution = execution(&record);
    match &request.command {
        ReceptionCommand::Claim {
            session_key,
            display_name,
            ..
        } => {
            if record.session_key != *session_key || record.display_name != *display_name {
                return Err(failure(RepositoryErrorKind::Conflict));
            }
            execution = execution
                .apply(ReceptionTransition::Claim(owner))
                .map_err(|_| failure(RepositoryErrorKind::Conflict))?;
            record.instance_id = request.instance_id;
            record.device_id = device.as_uuid();
            record.run_id = request.run_id;
        }
        ReceptionCommand::Release => {
            execution = execution
                .apply(ReceptionTransition::Release(owner))
                .map_err(|_| failure(RepositoryErrorKind::Conflict))?;
        }
        ReceptionCommand::Heartbeat | ReceptionCommand::Save { .. } => {
            if execution.owner != Some(owner) || execution.mode == ReceptionMode::Idle {
                return Err(failure(RepositoryErrorKind::Conflict));
            }
            if let ReceptionCommand::Save { revision, progress } = &request.command {
                // A response may be lost after commit. The identical state is safe to retry.
                if record.progress != *progress {
                    if record.revision != *revision {
                        return Err(failure(RepositoryErrorKind::Conflict));
                    }
                    record.progress = progress.clone();
                    record.revision = record
                        .revision
                        .checked_add(1)
                        .ok_or_else(|| failure(RepositoryErrorKind::Constraint))?;
                }
            }
        }
    }
    record.status = status(execution.mode);
    record.next_device_id = execution.next_device.map(DeviceId::as_uuid);
    save(tx, &record).await?;
    find(tx, request.agent_id, request.catalog_id)
        .await?
        .ok_or_else(|| failure(RepositoryErrorKind::CorruptData))
}
async fn create(
    tx: &mut Transaction<'_, Postgres>,
    device: DeviceId,
    request: &ReceptionRequest,
) -> RepositoryResult<ReceptionRecord> {
    let ReceptionCommand::Claim {
        session_key,
        display_name,
        initial,
    } = &request.command
    else {
        return Err(failure(RepositoryErrorKind::NotFound));
    };
    let matches: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_room.agent WHERE id=$1 AND slug=$2)")
            .bind(request.agent_id)
            .bind(format!("host-{session_key}"))
            .fetch_one(&mut **tx)
            .await
            .map_err(|error| map_sqlx_error(OP, &error))?;
    if !matches {
        return Err(failure(RepositoryErrorKind::Forbidden));
    }
    sqlx::query("INSERT INTO agent_room.reception_execution (agent_id,catalog_id,room_id,session_key,display_name,instance_id,device_id,run_id,state,last_seen_unix_ms,progress) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'active',floor(extract(epoch FROM clock_timestamp())*1000)::bigint,$9)")
        .bind(request.agent_id).bind(request.catalog_id).bind(&request.room_id).bind(session_key).bind(display_name)
        .bind(request.instance_id).bind(device.as_uuid()).bind(request.run_id).bind(serde_json::to_value(initial).map_err(|_| failure(RepositoryErrorKind::Constraint))?)
        .execute(&mut **tx).await.map_err(|error| map_sqlx_error(OP, &error))?;
    find(tx, request.agent_id, request.catalog_id)
        .await?
        .ok_or_else(|| failure(RepositoryErrorKind::CorruptData))
}

async fn lock_owned_agent(
    tx: &mut Transaction<'_, Postgres>,
    principal: PrincipalId,
    agent: Uuid,
) -> RepositoryResult<()> {
    let found = sqlx::query_scalar::<_, Uuid>("SELECT agent.id FROM agent_room.agent AS agent JOIN agent_room.agent_ownership AS ownership ON ownership.agent_id=agent.id WHERE agent.id=$1 AND ownership.principal_id=$2 AND ownership.role IN ('owner','operator') AND ownership.revoked_at IS NULL AND agent.lifecycle_state='active' FOR UPDATE OF agent")
        .bind(agent).bind(principal.as_uuid()).fetch_optional(&mut **tx).await.map_err(|error| map_sqlx_error(OP, &error))?;
    found
        .map(|_| ())
        .ok_or_else(|| failure(RepositoryErrorKind::Forbidden))
}
async fn find(
    tx: &mut Transaction<'_, Postgres>,
    agent: Uuid,
    catalog: Uuid,
) -> RepositoryResult<Option<ReceptionRecord>> {
    sqlx::query("SELECT reception.*, device.label AS device_label FROM agent_room.reception_execution AS reception JOIN agent_room.device AS device ON device.id = reception.device_id WHERE reception.agent_id=$1 AND reception.catalog_id=$2").bind(agent).bind(catalog).fetch_optional(&mut **tx).await.map_err(|error| map_sqlx_error(OP, &error))?.as_ref().map(decode).transpose()
}
async fn save(
    tx: &mut Transaction<'_, Postgres>,
    record: &ReceptionRecord,
) -> RepositoryResult<()> {
    sqlx::query("UPDATE agent_room.reception_execution SET instance_id=$3, device_id=$4, run_id=$5, state=$6, next_device_id=$7, revision=$8, progress=$9, last_seen_unix_ms=floor(extract(epoch FROM clock_timestamp())*1000)::bigint WHERE agent_id=$1 AND catalog_id=$2")
        .bind(record.agent_id).bind(record.catalog_id).bind(record.instance_id).bind(record.device_id).bind(record.run_id)
        .bind(status_text(record.status)).bind(record.next_device_id).bind(record.revision)
        .bind(serde_json::to_value(&record.progress).map_err(|_| failure(RepositoryErrorKind::Constraint))?)
        .execute(&mut **tx).await.map_err(|error| map_sqlx_error(OP, &error))?;
    Ok(())
}
fn execution(record: &ReceptionRecord) -> ReceptionExecution {
    ReceptionExecution {
        owner: Some(ReceptionOwner {
            instance_id: record.instance_id.into(),
            device_id: record.device_id.into(),
            run_id: record.run_id,
        }),
        mode: match record.status {
            ReceptionStatus::Active => ReceptionMode::Active,
            ReceptionStatus::Draining => ReceptionMode::Draining,
            ReceptionStatus::Idle => ReceptionMode::Idle,
        },
        next_device: record.next_device_id.map(DeviceId::from_uuid),
    }
}
const fn status(mode: ReceptionMode) -> ReceptionStatus {
    match mode {
        ReceptionMode::Active => ReceptionStatus::Active,
        ReceptionMode::Draining => ReceptionStatus::Draining,
        ReceptionMode::Idle => ReceptionStatus::Idle,
    }
}
const fn status_text(status: ReceptionStatus) -> &'static str {
    match status {
        ReceptionStatus::Active => "active",
        ReceptionStatus::Draining => "draining",
        ReceptionStatus::Idle => "idle",
    }
}
fn decode(row: &PgRow) -> RepositoryResult<ReceptionRecord> {
    let state: String = decode_column(row, "state", OP)?;
    let progress: ReceptionProgress = serde_json::from_value(decode_column(row, "progress", OP)?)
        .map_err(|_| failure(RepositoryErrorKind::CorruptData))?;
    if !progress.valid() {
        return Err(failure(RepositoryErrorKind::CorruptData));
    }
    Ok(ReceptionRecord {
        agent_id: decode_column(row, "agent_id", OP)?,
        catalog_id: decode_column(row, "catalog_id", OP)?,
        room_id: decode_column(row, "room_id", OP)?,
        session_key: decode_column(row, "session_key", OP)?,
        display_name: decode_column(row, "display_name", OP)?,
        instance_id: decode_column(row, "instance_id", OP)?,
        device_id: decode_column(row, "device_id", OP)?,
        device_label: decode_column(row, "device_label", OP)?,
        run_id: decode_column(row, "run_id", OP)?,
        status: match state.as_str() {
            "active" => ReceptionStatus::Active,
            "draining" => ReceptionStatus::Draining,
            "idle" => ReceptionStatus::Idle,
            _ => return Err(failure(RepositoryErrorKind::CorruptData)),
        },
        next_device_id: decode_column(row, "next_device_id", OP)?,
        last_seen_unix_ms: decode_column(row, "last_seen_unix_ms", OP)?,
        revision: decode_column(row, "revision", OP)?,
        progress,
    })
}
