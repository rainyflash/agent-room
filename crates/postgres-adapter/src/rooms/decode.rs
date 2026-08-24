use agent_room_application::persistence::{RepositoryError, RepositoryResult};
use agent_room_domain::{
    ids::{AgentInstanceId, PrincipalId, RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState, RoomLanguage, RoomRegion, RoomReservation, RoomReservationFields,
        RoomReservationState, RoomSlug,
    },
};
use sqlx::postgres::PgRow;

use crate::agents::{decode_column, decode_optional_time, decode_time};

pub(super) fn decode_catalog(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<RoomCatalog> {
    let id: uuid::Uuid = decode_column(row, "catalog_id", operation)?;
    let kind: String = decode_column(row, "catalog_kind", operation)?;
    let slug: Option<String> = decode_column(row, "catalog_slug", operation)?;
    let language: Option<String> = decode_column(row, "catalog_language", operation)?;
    let matrix_space_id: Option<String> = decode_column(row, "catalog_matrix_space_id", operation)?;
    let owner_principal_id: Option<uuid::Uuid> =
        decode_column(row, "catalog_owner_principal_id", operation)?;
    let visibility: String = decode_column(row, "catalog_visibility", operation)?;
    let retention_days: Option<i32> = decode_column(row, "catalog_retention_days", operation)?;
    let status: String = decode_column(row, "catalog_status", operation)?;

    RoomCatalog::new(
        RoomCatalogId::from_uuid(id),
        RoomCatalogFields {
            kind: RoomCatalogKind::try_from(kind.as_str()).map_err(|_| corrupt_data(operation))?,
            slug: slug
                .map(RoomSlug::new)
                .transpose()
                .map_err(|_| corrupt_data(operation))?,
            name: decode_column(row, "catalog_name", operation)?,
            description: decode_column(row, "catalog_description", operation)?,
            language: language
                .map(RoomLanguage::new)
                .transpose()
                .map_err(|_| corrupt_data(operation))?,
            matrix_space_id: matrix_space_id
                .map(MatrixRoomReference::new)
                .transpose()
                .map_err(|_| corrupt_data(operation))?,
            owner_principal_id: owner_principal_id.map(PrincipalId::from_uuid),
            visibility: RoomCatalogVisibility::try_from(visibility.as_str())
                .map_err(|_| corrupt_data(operation))?,
            retention_days: retention_days
                .map(u16::try_from)
                .transpose()
                .map_err(|_| corrupt_data(operation))?,
            status: RoomCatalogStatus::try_from(status.as_str())
                .map_err(|_| corrupt_data(operation))?,
        },
    )
    .map_err(|_| corrupt_data(operation))
}

pub(super) fn decode_instance(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<RoomInstance> {
    let id: uuid::Uuid = decode_column(row, "room_instance_id", operation)?;
    let catalog_id: uuid::Uuid = decode_column(row, "instance_catalog_id", operation)?;
    let matrix_room_id: String = decode_column(row, "matrix_room_id", operation)?;
    let region: Option<String> = decode_column(row, "region_hint", operation)?;
    let soft_capacity: i32 = decode_column(row, "soft_capacity", operation)?;
    let hard_capacity: i32 = decode_column(row, "hard_capacity", operation)?;
    let projected_member_count: i32 = decode_column(row, "member_count_projection", operation)?;
    let allocated_slots: i32 = decode_column(row, "allocated_slots", operation)?;
    let activity_score_millis: i64 = decode_column(row, "activity_score_millis", operation)?;
    let state: String = decode_column(row, "instance_state", operation)?;

    RoomInstance::restore(
        RoomInstanceId::from_uuid(id),
        RoomInstanceFields {
            catalog_id: RoomCatalogId::from_uuid(catalog_id),
            matrix_room_id: MatrixRoomReference::new(matrix_room_id)
                .map_err(|_| corrupt_data(operation))?,
            region: region
                .map(RoomRegion::new)
                .transpose()
                .map_err(|_| corrupt_data(operation))?,
            capacity: RoomCapacity::new(
                decode_u16(soft_capacity, operation)?,
                decode_u16(hard_capacity, operation)?,
            )
            .map_err(|_| corrupt_data(operation))?,
            projected_member_count: decode_u16(projected_member_count, operation)?,
            allocated_slots: decode_u16(allocated_slots, operation)?,
            activity_score_millis: u64::try_from(activity_score_millis)
                .map_err(|_| corrupt_data(operation))?,
            state: RoomInstanceState::try_from(state.as_str())
                .map_err(|_| corrupt_data(operation))?,
        },
    )
    .map_err(|_| corrupt_data(operation))
}

pub(super) fn decode_reservation(
    row: &PgRow,
    operation: &'static str,
) -> RepositoryResult<RoomReservation> {
    let id: uuid::Uuid = decode_column(row, "reservation_id", operation)?;
    let catalog_id: uuid::Uuid = decode_column(row, "reservation_catalog_id", operation)?;
    let room_instance_id: uuid::Uuid =
        decode_column(row, "reservation_room_instance_id", operation)?;
    let agent_instance_id: uuid::Uuid =
        decode_column(row, "reservation_agent_instance_id", operation)?;
    let state: String = decode_column(row, "reservation_state", operation)?;

    RoomReservation::restore(
        RoomReservationId::from_uuid(id),
        RoomReservationFields {
            catalog_id: RoomCatalogId::from_uuid(catalog_id),
            room_instance_id: RoomInstanceId::from_uuid(room_instance_id),
            agent_instance_id: AgentInstanceId::from_uuid(agent_instance_id),
            reserved_at: decode_time(row, "reserved_at_ms", operation)?,
            expires_at: decode_time(row, "expires_at_ms", operation)?,
            state: RoomReservationState::try_from(state.as_str())
                .map_err(|_| corrupt_data(operation))?,
            finalized_at: decode_optional_time(row, "finalized_at_ms", operation)?,
        },
    )
    .map_err(|_| corrupt_data(operation))
}

pub(super) const CATALOG_COLUMNS: &str = r"
    catalog.id AS catalog_id,
    catalog.kind AS catalog_kind,
    catalog.slug AS catalog_slug,
    catalog.name AS catalog_name,
    catalog.description AS catalog_description,
    catalog.language AS catalog_language,
    catalog.matrix_space_id AS catalog_matrix_space_id,
    catalog.owner_principal_id AS catalog_owner_principal_id,
    catalog.visibility AS catalog_visibility,
    catalog.retention_days AS catalog_retention_days,
    catalog.status AS catalog_status";

pub(super) const INSTANCE_COLUMNS: &str = r"
    instance.id AS room_instance_id,
    instance.catalog_entry_id AS instance_catalog_id,
    instance.matrix_room_id,
    instance.region_hint,
    instance.soft_capacity,
    instance.hard_capacity,
    instance.member_count_projection,
    instance.allocated_slots,
    floor(instance.activity_score * 1000)::bigint AS activity_score_millis,
    instance.state AS instance_state";

pub(super) const RESERVATION_COLUMNS: &str = r"
    reservation.id AS reservation_id,
    reservation.catalog_entry_id AS reservation_catalog_id,
    reservation.room_instance_id AS reservation_room_instance_id,
    reservation.agent_instance_id AS reservation_agent_instance_id,
    reservation.state AS reservation_state,
    floor(extract(epoch FROM reservation.reserved_at) * 1000)::bigint AS reserved_at_ms,
    floor(extract(epoch FROM reservation.expires_at) * 1000)::bigint AS expires_at_ms,
    floor(extract(epoch FROM reservation.finalized_at) * 1000)::bigint AS finalized_at_ms";

fn decode_u16(value: i32, operation: &'static str) -> RepositoryResult<u16> {
    u16::try_from(value).map_err(|_| corrupt_data(operation))
}

pub(super) const fn corrupt_data(operation: &'static str) -> RepositoryError {
    RepositoryError::new(
        operation,
        agent_room_application::persistence::RepositoryErrorKind::CorruptData,
    )
}
