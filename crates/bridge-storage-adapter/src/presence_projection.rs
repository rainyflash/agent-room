use std::{
    collections::{BTreeMap, BTreeSet},
    future::ready,
    sync::RwLock,
};

use agent_room_application::ports::{MatrixRoomId, MatrixUserId, PortFuture};
use agent_room_bridge_core::presence::{
    PresenceObservation, PresenceProjectionBatch, PresenceProjectionFailure,
    PresenceProjectionFailureKind, PresenceProjectionRepository, PresenceQuery,
    PresenceRoomProjectionMode, ProjectedAgentPresence,
};
use agent_room_domain::{agent_status::AgentWorkStatus, ids::AgentInstanceId};

const MAXIMUM_PRESENCE_RESULTS: usize = 250;

#[derive(Default)]
pub struct InMemoryPresenceProjectionRepository {
    state: RwLock<PresenceProjectionState>,
}

#[derive(Default)]
struct PresenceProjectionState {
    rooms: BTreeMap<MatrixRoomId, RoomPresenceState>,
}

#[derive(Default)]
struct RoomPresenceState {
    joined_members: BTreeSet<MatrixUserId>,
    instances: BTreeMap<AgentInstanceId, ProjectedAgentPresence>,
}

impl PresenceProjectionRepository for InMemoryPresenceProjectionRepository {
    fn apply<'a>(
        &'a self,
        batch: &'a PresenceProjectionBatch,
    ) -> PortFuture<'a, Result<(), PresenceProjectionFailure>> {
        let result = self.apply_sync(batch);
        Box::pin(ready(result))
    }

    fn list<'a>(
        &'a self,
        query: &'a PresenceQuery,
    ) -> PortFuture<'a, Result<Vec<PresenceObservation>, PresenceProjectionFailure>> {
        let result = self.list_sync(query);
        Box::pin(ready(result))
    }
}

impl InMemoryPresenceProjectionRepository {
    fn apply_sync(&self, batch: &PresenceProjectionBatch) -> Result<(), PresenceProjectionFailure> {
        let mut state = self.state.write().map_err(|_| corrupt_projection())?;
        for update in batch.rooms() {
            if update.mode() == PresenceRoomProjectionMode::Remove {
                state.rooms.remove(update.room_id());
                continue;
            }
            let room = state.rooms.entry(update.room_id().clone()).or_default();
            if update.mode() == PresenceRoomProjectionMode::Replace {
                room.joined_members.clear();
                room.instances.clear();
            }
            for membership in update.memberships() {
                if membership.joined() {
                    room.joined_members
                        .insert(membership.matrix_user_id().clone());
                } else {
                    room.joined_members.remove(membership.matrix_user_id());
                }
            }
            room.instances.retain(|_, presence| {
                room.joined_members
                    .contains(presence.identity().matrix_user_id())
            });
            for presence in update.presences() {
                if !room
                    .joined_members
                    .contains(presence.identity().matrix_user_id())
                {
                    continue;
                }
                let instance_id = presence.identity().agent_instance_id();
                let replace = room
                    .instances
                    .get(&instance_id)
                    .is_none_or(|current| status_is_newer(presence, current));
                if replace {
                    room.instances.insert(instance_id, presence.clone());
                }
            }
        }
        Ok(())
    }

    fn list_sync(
        &self,
        query: &PresenceQuery,
    ) -> Result<Vec<PresenceObservation>, PresenceProjectionFailure> {
        let state = self.state.read().map_err(|_| corrupt_projection())?;
        let Some(room) = state.rooms.get(query.room_id()) else {
            return Ok(Vec::new());
        };
        let mut presences = room
            .instances
            .values()
            .filter(|presence| {
                query.agent_ids().is_empty()
                    || query.agent_ids().contains(&presence.identity().agent_id())
            })
            .cloned()
            .collect::<Vec<_>>();
        presences.sort_by_key(|presence| {
            (
                presence.identity().agent_id(),
                presence.identity().agent_instance_id(),
            )
        });
        presences.truncate(MAXIMUM_PRESENCE_RESULTS);
        Ok(presences
            .into_iter()
            .map(|presence| {
                let status = if presence.status() == AgentWorkStatus::Offline
                    || query.observed_at() >= presence.lease_expires_at()
                {
                    AgentWorkStatus::Offline
                } else {
                    presence.status()
                };
                PresenceObservation::new(presence, status, query.observed_at())
            })
            .collect())
    }
}

fn status_is_newer(candidate: &ProjectedAgentPresence, current: &ProjectedAgentPresence) -> bool {
    candidate.origin_server_timestamp() > current.origin_server_timestamp()
        || (candidate.origin_server_timestamp() == current.origin_server_timestamp()
            && candidate.event_id().as_str() > current.event_id().as_str())
}

const fn corrupt_projection() -> PresenceProjectionFailure {
    PresenceProjectionFailure::new(PresenceProjectionFailureKind::Corrupt)
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{MatrixEventId, MatrixRoomId, MatrixUserId};
    use agent_room_bridge_core::{
        agent_identity::BridgeAgentIdentity,
        presence::{
            PresenceMembershipChange, PresenceProjectionBatch, PresenceQuery,
            PresenceRoomProjection, PresenceRoomProjectionMode, ProjectedAgentPresence,
            ProjectedAgentPresenceFields,
        },
    };
    use agent_room_domain::{
        agent_status::AgentWorkStatus,
        ids::{AgentId, AgentInstanceId},
        time::UtcMillis,
    };
    use uuid::Uuid;

    use super::InMemoryPresenceProjectionRepository;

    const AGENT_ID: &str = "01945c1e-7b5a-7c7f-8a28-2de53f56a9a3";
    const INSTANCE_ID: &str = "01945c1e-7b5a-7c7f-8a28-2de53f56a9a4";
    const MATRIX_USER_ID: &str = "@agent:matrix.test";

    #[test]
    fn 成员离房会立即清除该用户全部状态() {
        let repository = InMemoryPresenceProjectionRepository::default();
        repository
            .apply_sync(&replace_batch(presence(
                "$working:matrix.test",
                AgentWorkStatus::Working,
                10,
                2_000,
            )))
            .expect("初始状态可投影");
        repository
            .apply_sync(&PresenceProjectionBatch::new(vec![
                PresenceRoomProjection::new(
                    room_id(),
                    PresenceRoomProjectionMode::Delta,
                    vec![PresenceMembershipChange::new(matrix_user_id(), false)],
                    Vec::new(),
                ),
            ]))
            .expect("离房状态可投影");

        assert!(
            repository
                .list_sync(&query(1_500))
                .expect("Presence 可查询")
                .is_empty()
        );
    }

    #[test]
    fn 租约过期在读取时本地降级为离线() {
        let repository = InMemoryPresenceProjectionRepository::default();
        repository
            .apply_sync(&replace_batch(presence(
                "$working:matrix.test",
                AgentWorkStatus::Working,
                10,
                2_000,
            )))
            .expect("初始状态可投影");

        let observations = repository
            .list_sync(&query(2_000))
            .expect("Presence 可查询");

        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].status(), AgentWorkStatus::Offline);
    }

    #[test]
    fn 乱序到达的旧状态不能覆盖较新的状态() {
        let repository = InMemoryPresenceProjectionRepository::default();
        repository
            .apply_sync(&replace_batch(presence(
                "$new:matrix.test",
                AgentWorkStatus::Blocked,
                20,
                3_000,
            )))
            .expect("新状态可投影");
        repository
            .apply_sync(&PresenceProjectionBatch::new(vec![
                PresenceRoomProjection::new(
                    room_id(),
                    PresenceRoomProjectionMode::Delta,
                    Vec::new(),
                    vec![presence(
                        "$old:matrix.test",
                        AgentWorkStatus::Working,
                        10,
                        3_000,
                    )],
                ),
            ]))
            .expect("乱序状态批次可处理");

        let observations = repository
            .list_sync(&query(1_500))
            .expect("Presence 可查询");

        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].status(), AgentWorkStatus::Blocked);
        assert_eq!(
            observations[0].presence().event_id().as_str(),
            "$new:matrix.test"
        );
    }

    fn replace_batch(presence: ProjectedAgentPresence) -> PresenceProjectionBatch {
        PresenceProjectionBatch::new(vec![PresenceRoomProjection::new(
            room_id(),
            PresenceRoomProjectionMode::Replace,
            vec![PresenceMembershipChange::new(matrix_user_id(), true)],
            vec![presence],
        )])
    }

    fn presence(
        event_id: &str,
        status: AgentWorkStatus,
        origin_server_timestamp: u64,
        lease_expires_at: i64,
    ) -> ProjectedAgentPresence {
        ProjectedAgentPresence::from_verified_fields(ProjectedAgentPresenceFields {
            event_id: MatrixEventId::new(event_id).expect("事件标识有效"),
            room_id: room_id(),
            identity: BridgeAgentIdentity::new(
                AgentId::from_uuid(Uuid::parse_str(AGENT_ID).expect("Agent ID 有效")),
                "Presence Agent",
                MATRIX_USER_ID,
                AgentInstanceId::from_uuid(Uuid::parse_str(INSTANCE_ID).expect("实例 ID 有效")),
            )
            .expect("公开身份有效"),
            status,
            observed_at: UtcMillis::new(1_000).expect("观察时间有效"),
            lease_expires_at: UtcMillis::new(lease_expires_at).expect("租约时间有效"),
            origin_server_timestamp,
        })
    }

    fn query(observed_at: i64) -> PresenceQuery {
        PresenceQuery::new(
            room_id(),
            Vec::new(),
            UtcMillis::new(observed_at).expect("查询时间有效"),
        )
        .expect("Presence 查询有效")
    }

    fn room_id() -> MatrixRoomId {
        MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效")
    }

    fn matrix_user_id() -> MatrixUserId {
        MatrixUserId::new(MATRIX_USER_ID).expect("Matrix 用户标识有效")
    }
}
