use std::collections::BTreeMap;

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        PortFuture, RoomAllocationMode, RoomAllocationStore, RoomReservationClaim,
        RoomReservationOutcome,
    },
};
use agent_room_domain::{
    ids::{RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{
        RoomAllocationAffinity, RoomAllocationCandidate, RoomAllocationDecision, RoomCatalog,
        RoomCatalogKind, RoomCatalogVisibility, RoomInstance, RoomInvitationAffinity,
        RoomPreferenceMatch, RoomRecoveryAffinity, RoomReservation, RoomReservationState,
        choose_manual_room_instance, choose_room_instance,
    },
    time::UtcMillis,
};
use sqlx::{Postgres, Transaction, postgres::PgRow};

use crate::{
    PostgresRepositories,
    agents::decode_column,
    error::{map_domain_error, map_sqlx_error},
    transaction::finish,
};

use super::decode::{
    CATALOG_COLUMNS, INSTANCE_COLUMNS, RESERVATION_COLUMNS, corrupt_data, decode_catalog,
    decode_instance, decode_reservation,
};

impl RoomAllocationStore for PostgresRepositories {
    fn reserve<'a>(
        &'a self,
        claim: &'a RoomReservationClaim,
    ) -> PortFuture<'a, RepositoryResult<RoomReservationOutcome>> {
        Box::pin(async move { self.reserve_room(claim).await })
    }

    fn transition(
        &self,
        reservation_id: RoomReservationId,
        expected: RoomReservationState,
        target: RoomReservationState,
        changed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<RoomReservation>> {
        Box::pin(async move {
            self.transition_reservation(reservation_id, expected, target, changed_at)
                .await
        })
    }

    fn expire_pending(&self, now: UtcMillis, limit: u16) -> PortFuture<'_, RepositoryResult<u16>> {
        Box::pin(async move { self.expire_room_reservations(now, limit).await })
    }
}

impl PostgresRepositories {
    async fn reserve_room(
        &self,
        claim: &RoomReservationClaim,
    ) -> RepositoryResult<RoomReservationOutcome> {
        let operation = "room_allocation.reserve";
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;

        let result = async {
            authorize_agent_instance(&mut transaction, claim).await?;
            let catalog = lock_catalog(&mut transaction, claim.catalog_id).await?;
            ensure_allocatable_catalog(&catalog, operation)?;

            if let Some(stored) =
                lock_reservation(&mut transaction, claim.reservation_id, operation).await?
            {
                return replay_reservation(&mut transaction, claim, stored, operation).await;
            }

            let current = lock_current_assignment(
                &mut transaction,
                claim.agent_instance_id,
                claim.catalog_id,
                None,
                operation,
            )
            .await?;
            if let Some(stored) = current
                && assignment_satisfies_mode(&stored.reservation, claim.mode)
            {
                let room = lock_instance(
                    &mut transaction,
                    stored.reservation.room_instance_id(),
                    operation,
                )
                .await?;
                return Ok(RoomReservationOutcome::ExistingAssignment {
                    reservation: stored.reservation,
                    room,
                });
            }

            ensure_no_pending_assignment(&mut transaction, claim, operation).await?;
            let candidates = lock_candidates(&mut transaction, &catalog, claim).await?;
            let decision = choose_candidate(&candidates, claim.mode, operation)?;
            let RoomAllocationDecision::Reserve(selected_id) = decision else {
                return Ok(RoomReservationOutcome::ProvisioningRequired { catalog });
            };
            let mut selected = candidates
                .into_iter()
                .find(|candidate| candidate.instance().id() == selected_id)
                .map(|candidate| candidate.instance().clone())
                .ok_or_else(|| corrupt_data(operation))?;
            selected
                .reserve_slot()
                .map_err(|error| map_domain_error(operation, &error))?;
            let reservation = RoomReservation::reserve(
                claim.reservation_id,
                claim.catalog_id,
                selected.id(),
                claim.agent_instance_id,
                claim.reserved_at,
                claim.expires_at,
            )
            .map_err(|error| map_domain_error(operation, &error))?;

            persist_room_slots(&mut transaction, &selected, claim.reserved_at, operation).await?;
            insert_reservation(&mut transaction, claim, &reservation, operation).await?;
            Ok(RoomReservationOutcome::Reserved {
                reservation,
                room: selected,
            })
        }
        .await;

        finish(transaction, result, operation).await
    }

    async fn transition_reservation(
        &self,
        reservation_id: RoomReservationId,
        expected: RoomReservationState,
        target: RoomReservationState,
        changed_at: UtcMillis,
    ) -> RepositoryResult<RoomReservation> {
        let operation = "room_allocation.transition";
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;

        let result = async {
            let mut stored = lock_reservation(&mut transaction, reservation_id, operation)
                .await?
                .ok_or_else(|| not_found(operation))?;
            if stored.reservation.state() == target {
                return Ok(stored.reservation);
            }
            if stored.reservation.state() != expected {
                return Err(conflict(operation));
            }

            match target {
                RoomReservationState::Committed => {
                    commit_reservation(&mut transaction, &mut stored, changed_at, operation)
                        .await?;
                }
                RoomReservationState::Released | RoomReservationState::Expired => {
                    finalize_reservation(
                        &mut transaction,
                        &mut stored.reservation,
                        target,
                        changed_at,
                        operation,
                    )
                    .await?;
                }
                RoomReservationState::Reserved => return Err(constraint(operation)),
            }
            Ok(stored.reservation)
        }
        .await;

        finish(transaction, result, operation).await
    }

    async fn expire_room_reservations(&self, now: UtcMillis, limit: u16) -> RepositoryResult<u16> {
        let operation = "room_allocation.expire_pending";
        let mut transaction = self
            .pool()
            .begin()
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;

        let result = async {
            let reservations =
                lock_expired_reservations(&mut transaction, now, limit, operation).await?;
            if reservations.is_empty() {
                return Ok(0);
            }
            let room_ids = reservations
                .iter()
                .map(|stored| stored.reservation.room_instance_id())
                .collect::<Vec<_>>();
            let mut rooms = lock_instances(&mut transaction, &room_ids, operation).await?;

            for mut stored in reservations {
                let changed = stored
                    .reservation
                    .expire(now)
                    .map_err(|_| corrupt_data(operation))?;
                if !changed {
                    return Err(corrupt_data(operation));
                }
                let room = rooms
                    .get_mut(&stored.reservation.room_instance_id())
                    .ok_or_else(|| corrupt_data(operation))?;
                room.release_slot().map_err(|_| corrupt_data(operation))?;
                persist_reservation(&mut transaction, &stored.reservation, operation).await?;
            }
            for room in rooms.values() {
                persist_room_slots(&mut transaction, room, now, operation).await?;
            }

            u16::try_from(room_ids.len()).map_err(|_| corrupt_data(operation))
        }
        .await;

        finish(transaction, result, operation).await
    }
}

#[derive(Debug)]
struct StoredReservation {
    reservation: RoomReservation,
    agent_id: agent_room_domain::ids::AgentId,
}

async fn authorize_agent_instance(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &RoomReservationClaim,
) -> RepositoryResult<()> {
    let operation = "room_allocation.authorize";
    let authorized: bool = sqlx::query_scalar(
        r"SELECT EXISTS (
              SELECT 1
              FROM agent_room.agent_instance AS instance
              JOIN agent_room.agent AS agent ON agent.id = instance.agent_id
              WHERE instance.id = $1
                AND instance.agent_id = $2
                AND instance.revoked_at IS NULL
                AND instance.status <> 'revoked'
                AND agent.lifecycle_state = 'active'
          )",
    )
    .bind(claim.agent_instance_id.as_uuid())
    .bind(claim.agent_id.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if authorized {
        Ok(())
    } else {
        Err(RepositoryError::new(
            operation,
            RepositoryErrorKind::Forbidden,
        ))
    }
}

async fn lock_catalog(
    transaction: &mut Transaction<'_, Postgres>,
    catalog_id: RoomCatalogId,
) -> RepositoryResult<RoomCatalog> {
    let operation = "room_allocation.catalog";
    let statement = format!(
        r"SELECT {CATALOG_COLUMNS}
           FROM agent_room.room_catalog_entry AS catalog
           WHERE catalog.id = $1
           FOR SHARE"
    );
    // 这里只拼接编译期固定列清单，所有运行时值仍通过参数绑定。
    let row = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(catalog_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?
        .ok_or_else(|| not_found(operation))?;
    decode_catalog(&row, operation)
}

fn ensure_allocatable_catalog(
    catalog: &RoomCatalog,
    operation: &'static str,
) -> RepositoryResult<()> {
    if catalog.kind() == RoomCatalogKind::PublicLobby
        && catalog.visibility() != RoomCatalogVisibility::Private
        && catalog.is_joinable()
    {
        Ok(())
    } else {
        Err(RepositoryError::new(
            operation,
            RepositoryErrorKind::Forbidden,
        ))
    }
}

async fn replay_reservation(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &RoomReservationClaim,
    stored: StoredReservation,
    operation: &'static str,
) -> RepositoryResult<RoomReservationOutcome> {
    if stored.agent_id != claim.agent_id
        || stored.reservation.agent_instance_id() != claim.agent_instance_id
        || stored.reservation.catalog_id() != claim.catalog_id
        || stored.reservation.reserved_at() != claim.reserved_at
        || stored.reservation.expires_at() != claim.expires_at
    {
        return Err(conflict(operation));
    }
    let room = lock_instance(
        transaction,
        stored.reservation.room_instance_id(),
        operation,
    )
    .await?;
    match stored.reservation.state() {
        RoomReservationState::Reserved => Ok(RoomReservationOutcome::Reserved {
            reservation: stored.reservation,
            room,
        }),
        RoomReservationState::Committed => Ok(RoomReservationOutcome::ExistingAssignment {
            reservation: stored.reservation,
            room,
        }),
        RoomReservationState::Released | RoomReservationState::Expired => Err(conflict(operation)),
    }
}

fn assignment_satisfies_mode(reservation: &RoomReservation, mode: RoomAllocationMode) -> bool {
    match mode {
        RoomAllocationMode::Automatic => true,
        RoomAllocationMode::Manual(target) => {
            reservation.room_instance_id().as_uuid() == target.as_uuid()
        }
    }
}

async fn ensure_no_pending_assignment(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &RoomReservationClaim,
    operation: &'static str,
) -> RepositoryResult<()> {
    let pending: Option<uuid::Uuid> = sqlx::query_scalar(
        r"SELECT id
           FROM agent_room.room_capacity_reservation
           WHERE agent_instance_id = $1
             AND catalog_entry_id = $2
             AND state = 'reserved'
           FOR UPDATE",
    )
    .bind(claim.agent_instance_id.as_uuid())
    .bind(claim.catalog_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if pending.is_some() {
        Err(conflict(operation))
    } else {
        Ok(())
    }
}

async fn lock_candidates(
    transaction: &mut Transaction<'_, Postgres>,
    catalog: &RoomCatalog,
    claim: &RoomReservationClaim,
) -> RepositoryResult<Vec<RoomAllocationCandidate>> {
    let operation = "room_allocation.candidates";
    let (filter, target) = match claim.mode {
        RoomAllocationMode::Automatic => ("instance.state = 'active'", None),
        RoomAllocationMode::Manual(target) => ("instance.id = $2", Some(target.as_uuid())),
    };
    let statement = format!(
        r"SELECT {INSTANCE_COLUMNS}
           FROM agent_room.room_instance AS instance
           WHERE instance.catalog_entry_id = $1
             AND {filter}
           ORDER BY instance.id
           FOR UPDATE"
    );
    // 过滤片段来自封闭枚举，不包含任何用户输入。
    let mut query = sqlx::query(sqlx::AssertSqlSafe(statement)).bind(catalog.id().as_uuid());
    if let Some(target) = target {
        query = query.bind(target);
    }
    let rows = query
        .fetch_all(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    rows.iter()
        .map(|row| {
            let instance = decode_instance(row, operation)?;
            Ok(RoomAllocationCandidate::new(
                instance.clone(),
                build_affinity(catalog, &instance, claim),
            ))
        })
        .collect()
}

fn build_affinity(
    catalog: &RoomCatalog,
    instance: &RoomInstance,
    claim: &RoomReservationClaim,
) -> RoomAllocationAffinity {
    RoomAllocationAffinity {
        recovery: if claim.evidence.previous_instance == Some(instance.id()) {
            RoomRecoveryAffinity::Previous
        } else {
            RoomRecoveryAffinity::Other
        },
        friends_in_room: claim
            .evidence
            .friends_per_instance
            .get(&instance.id())
            .copied()
            .unwrap_or_default(),
        invitation: if claim.evidence.invited_instances.contains(&instance.id()) {
            RoomInvitationAffinity::Explicit
        } else {
            RoomInvitationAffinity::None
        },
        language: preference_match(claim.preferred_language.as_ref(), catalog.language()),
        region: preference_match(claim.preferred_region.as_ref(), instance.region()),
    }
}

fn preference_match<T: PartialEq>(
    preferred: Option<&T>,
    actual: Option<&T>,
) -> RoomPreferenceMatch {
    if preferred.is_some() && preferred == actual {
        RoomPreferenceMatch::Matching
    } else {
        RoomPreferenceMatch::Different
    }
}

fn choose_candidate(
    candidates: &[RoomAllocationCandidate],
    mode: RoomAllocationMode,
    operation: &'static str,
) -> RepositoryResult<RoomAllocationDecision> {
    match mode {
        RoomAllocationMode::Automatic => Ok(choose_room_instance(candidates)),
        RoomAllocationMode::Manual(target) => {
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.instance().id() == target)
                .ok_or_else(|| not_found(operation))?;
            choose_manual_room_instance(candidate)
                .map_err(|error| map_domain_error(operation, &error))
        }
    }
}

async fn insert_reservation(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &RoomReservationClaim,
    reservation: &RoomReservation,
    operation: &'static str,
) -> RepositoryResult<()> {
    sqlx::query(
        r"INSERT INTO agent_room.room_capacity_reservation (
              id, catalog_entry_id, room_instance_id, agent_id, agent_instance_id,
              state, reserved_at, expires_at, finalized_at
          ) VALUES (
              $1, $2, $3, $4, $5, $6,
              to_timestamp($7::double precision / 1000.0),
              to_timestamp($8::double precision / 1000.0), NULL
          )",
    )
    .bind(reservation.id().as_uuid())
    .bind(reservation.catalog_id().as_uuid())
    .bind(reservation.room_instance_id().as_uuid())
    .bind(claim.agent_id.as_uuid())
    .bind(reservation.agent_instance_id().as_uuid())
    .bind(reservation.state().as_str())
    .bind(reservation.reserved_at().value())
    .bind(reservation.expires_at().value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    Ok(())
}

async fn commit_reservation(
    transaction: &mut Transaction<'_, Postgres>,
    stored: &mut StoredReservation,
    changed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let mut previous = lock_current_assignment(
        transaction,
        stored.reservation.agent_instance_id(),
        stored.reservation.catalog_id(),
        Some(stored.reservation.id()),
        operation,
    )
    .await?;
    let mut room_ids = vec![stored.reservation.room_instance_id()];
    if let Some(previous) = previous.as_ref() {
        room_ids.push(previous.reservation.room_instance_id());
    }
    let mut rooms = lock_instances(transaction, &room_ids, operation).await?;

    stored
        .reservation
        .commit(changed_at)
        .map_err(|error| map_domain_error(operation, &error))?;
    if let Some(previous) = previous.as_mut() {
        let changed = previous
            .reservation
            .release(changed_at)
            .map_err(|_| corrupt_data(operation))?;
        if !changed {
            return Err(corrupt_data(operation));
        }
        let previous_room = rooms
            .get_mut(&previous.reservation.room_instance_id())
            .ok_or_else(|| corrupt_data(operation))?;
        previous_room
            .release_slot()
            .map_err(|_| corrupt_data(operation))?;
        persist_reservation(transaction, &previous.reservation, operation).await?;
        persist_room_slots(transaction, previous_room, changed_at, operation).await?;
    }
    persist_reservation(transaction, &stored.reservation, operation).await
}

async fn finalize_reservation(
    transaction: &mut Transaction<'_, Postgres>,
    reservation: &mut RoomReservation,
    target: RoomReservationState,
    changed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let mut room = lock_instance(transaction, reservation.room_instance_id(), operation).await?;
    let changed = match target {
        RoomReservationState::Released => reservation.release(changed_at),
        RoomReservationState::Expired => reservation.expire(changed_at),
        RoomReservationState::Reserved | RoomReservationState::Committed => {
            return Err(constraint(operation));
        }
    }
    .map_err(|error| map_domain_error(operation, &error))?;
    if changed {
        room.release_slot().map_err(|_| corrupt_data(operation))?;
        persist_room_slots(transaction, &room, changed_at, operation).await?;
    }
    persist_reservation(transaction, reservation, operation).await
}

async fn persist_reservation(
    transaction: &mut Transaction<'_, Postgres>,
    reservation: &RoomReservation,
    operation: &'static str,
) -> RepositoryResult<()> {
    let finalized_at = reservation.finalized_at().map(UtcMillis::value);
    let result = sqlx::query(
        r"UPDATE agent_room.room_capacity_reservation
           SET state = $2,
               finalized_at = to_timestamp($3::double precision / 1000.0)
           WHERE id = $1",
    )
    .bind(reservation.id().as_uuid())
    .bind(reservation.state().as_str())
    .bind(finalized_at)
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(not_found(operation))
    }
}

async fn persist_room_slots(
    transaction: &mut Transaction<'_, Postgres>,
    room: &RoomInstance,
    changed_at: UtcMillis,
    operation: &'static str,
) -> RepositoryResult<()> {
    let result = sqlx::query(
        r"UPDATE agent_room.room_instance
           SET allocated_slots = $2,
               updated_at = to_timestamp($3::double precision / 1000.0),
               version = version + 1
           WHERE id = $1",
    )
    .bind(room.id().as_uuid())
    .bind(i32::from(room.allocated_slots()))
    .bind(changed_at.value())
    .execute(&mut **transaction)
    .await
    .map_err(|error| map_sqlx_error(operation, &error))?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(not_found(operation))
    }
}

async fn lock_reservation(
    transaction: &mut Transaction<'_, Postgres>,
    reservation_id: RoomReservationId,
    operation: &'static str,
) -> RepositoryResult<Option<StoredReservation>> {
    let statement = format!(
        r"SELECT {RESERVATION_COLUMNS},
                  reservation.agent_id AS reservation_agent_id
           FROM agent_room.room_capacity_reservation AS reservation
           WHERE reservation.id = $1
           FOR UPDATE"
    );
    // 这里只拼接编译期固定列清单，所有运行时值仍通过参数绑定。
    let row = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(reservation_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    row.as_ref()
        .map(|row| decode_stored_reservation(row, operation))
        .transpose()
}

async fn lock_current_assignment(
    transaction: &mut Transaction<'_, Postgres>,
    agent_instance_id: agent_room_domain::ids::AgentInstanceId,
    catalog_id: RoomCatalogId,
    excluded: Option<RoomReservationId>,
    operation: &'static str,
) -> RepositoryResult<Option<StoredReservation>> {
    let statement = format!(
        r"SELECT {RESERVATION_COLUMNS},
                  reservation.agent_id AS reservation_agent_id
           FROM agent_room.room_capacity_reservation AS reservation
           WHERE reservation.agent_instance_id = $1
             AND reservation.catalog_entry_id = $2
             AND reservation.state = 'committed'
             AND ($3::uuid IS NULL OR reservation.id <> $3)
           FOR UPDATE"
    );
    // 这里只拼接编译期固定列清单，所有运行时值仍通过参数绑定。
    let row = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(agent_instance_id.as_uuid())
        .bind(catalog_id.as_uuid())
        .bind(excluded.map(RoomReservationId::as_uuid))
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    row.as_ref()
        .map(|row| decode_stored_reservation(row, operation))
        .transpose()
}

async fn lock_expired_reservations(
    transaction: &mut Transaction<'_, Postgres>,
    now: UtcMillis,
    limit: u16,
    operation: &'static str,
) -> RepositoryResult<Vec<StoredReservation>> {
    let statement = format!(
        r"SELECT {RESERVATION_COLUMNS},
                  reservation.agent_id AS reservation_agent_id
           FROM agent_room.room_capacity_reservation AS reservation
           WHERE reservation.state = 'reserved'
             AND reservation.expires_at <= to_timestamp($1::double precision / 1000.0)
           ORDER BY reservation.expires_at, reservation.id
           LIMIT $2
           FOR UPDATE SKIP LOCKED"
    );
    // 这里只拼接编译期固定列清单，所有运行时值仍通过参数绑定。
    let rows = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(now.value())
        .bind(i64::from(limit))
        .fetch_all(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    rows.iter()
        .map(|row| decode_stored_reservation(row, operation))
        .collect()
}

fn decode_stored_reservation(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<StoredReservation> {
    let agent_id: uuid::Uuid = decode_column(row, "reservation_agent_id", operation)?;
    Ok(StoredReservation {
        reservation: decode_reservation(row, operation)?,
        agent_id: agent_room_domain::ids::AgentId::from_uuid(agent_id),
    })
}

async fn lock_instance(
    transaction: &mut Transaction<'_, Postgres>,
    room_id: RoomInstanceId,
    operation: &'static str,
) -> RepositoryResult<RoomInstance> {
    let mut rooms = lock_instances(transaction, &[room_id], operation).await?;
    rooms.remove(&room_id).ok_or_else(|| not_found(operation))
}

async fn lock_instances(
    transaction: &mut Transaction<'_, Postgres>,
    room_ids: &[RoomInstanceId],
    operation: &'static str,
) -> RepositoryResult<BTreeMap<RoomInstanceId, RoomInstance>> {
    let ids = room_ids.iter().map(|id| id.as_uuid()).collect::<Vec<_>>();
    let statement = format!(
        r"SELECT {INSTANCE_COLUMNS}
           FROM agent_room.room_instance AS instance
           WHERE instance.id = ANY($1::uuid[])
           ORDER BY instance.id
           FOR UPDATE"
    );
    // 这里只拼接编译期固定列清单，所有运行时值仍通过参数绑定。
    let rows = sqlx::query(sqlx::AssertSqlSafe(statement))
        .bind(&ids)
        .fetch_all(&mut **transaction)
        .await
        .map_err(|error| map_sqlx_error(operation, &error))?;
    let mut rooms = BTreeMap::new();
    for row in &rows {
        let room = decode_instance(row, operation)?;
        rooms.insert(room.id(), room);
    }
    if room_ids.iter().all(|id| rooms.contains_key(id)) {
        Ok(rooms)
    } else {
        Err(not_found(operation))
    }
}

const fn conflict(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Conflict)
}

const fn constraint(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Constraint)
}

const fn not_found(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::NotFound)
}
