use std::{
    collections::{BTreeMap, BTreeSet},
    future::ready,
    sync::RwLock,
};

use agent_room_application::ports::{MatrixRoomId, MatrixUserId, MatrixUserPresence, PortFuture};
use agent_room_bridge_core::presence::{
    PresenceObservation, PresenceProjectionBatch, PresenceProjectionFailure,
    PresenceProjectionFailureKind, PresenceProjectionRepository, PresenceQuery,
    PresenceRoomProjectionMode, ProjectedAgentPresence,
};
use agent_room_bridge_core::{
    presence_roster::project_roster_with_presence, presence_tracker::MatrixPresenceTracker,
};
use agent_room_domain::{
    agent_lifecycle::AgentRosterPolicy, ids::AgentInstanceId, time::UtcMillis,
};

#[derive(Default)]
pub struct InMemoryPresenceProjectionRepository {
    state: RwLock<PresenceProjectionState>,
}

#[derive(Default)]
struct PresenceProjectionState {
    rooms: BTreeMap<MatrixRoomId, RoomPresenceState>,
    /// Matrix 在线状态按用户记，不按房间。
    presence: MatrixPresenceTracker,
}

#[derive(Default)]
struct RoomPresenceState {
    joined_members: BTreeSet<MatrixUserId>,
    instances: BTreeMap<AgentInstanceId, ProjectedAgentPresence>,
    policy: AgentRosterPolicy,
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

    fn presence_wanted(
        &self,
        now: UtcMillis,
        limit: usize,
    ) -> PortFuture<'_, Result<Vec<MatrixUserId>, PresenceProjectionFailure>> {
        let result = self.presence_wanted_sync(now, limit);
        Box::pin(ready(result))
    }

    fn record_fetched_presence<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
        presence: Option<&'a MatrixUserPresence>,
        fetched_at: UtcMillis,
    ) -> PortFuture<'a, Result<(), PresenceProjectionFailure>> {
        let result = self
            .state
            .write()
            .map_err(|_| corrupt_projection())
            .map(|mut state| {
                state.presence.record_fetched(user_id, presence, fetched_at);
            });
        Box::pin(ready(result))
    }
}

impl InMemoryPresenceProjectionRepository {
    fn apply_sync(&self, batch: &PresenceProjectionBatch) -> Result<(), PresenceProjectionFailure> {
        let mut state = self.state.write().map_err(|_| corrupt_projection())?;
        let state = &mut *state;
        if let Some(update) = batch.presence() {
            state.presence.apply_sync(update);
        }
        for update in batch.rooms() {
            if update.mode() == PresenceRoomProjectionMode::Remove {
                state.rooms.remove(update.room_id());
                continue;
            }
            let room = state.rooms.entry(update.room_id().clone()).or_default();
            if update.mode() == PresenceRoomProjectionMode::Replace {
                room.joined_members.clear();
                room.instances.clear();
                room.policy = AgentRosterPolicy::default();
            }
            if let Some(policy) = update.policy() {
                room.policy = policy;
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
            compact_disconnected_instances(room, &state.presence);
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
        Ok(project_roster_with_presence(
            room.instances.values(),
            query.observed_at(),
            room.policy,
            |user_id| state.presence.observation(user_id),
        )
        .into_iter()
        .filter(|entry| {
            query.agent_ids().is_empty()
                || query
                    .agent_ids()
                    .contains(&entry.presence().identity().agent_id())
        })
        .collect())
    }

    /// 还在房间里、写名片、还没拿到在线状态的人。离开的人名片还留在房间状态里，问不到也用不着。
    fn presence_wanted_sync(
        &self,
        now: UtcMillis,
        limit: usize,
    ) -> Result<Vec<MatrixUserId>, PresenceProjectionFailure> {
        let state = self.state.read().map_err(|_| corrupt_projection())?;
        let wanted = state
            .rooms
            .values()
            .flat_map(|room| room.instances.values())
            .filter(|presence| presence.is_card())
            .map(|presence| presence.identity().matrix_user_id())
            .filter(|user_id| state.presence.wants(user_id, now))
            .collect::<BTreeSet<_>>();
        Ok(wanted.into_iter().take(limit).cloned().collect())
    }
}

/// Preserve current sessions and one historical identity record, not every past process.
fn compact_disconnected_instances(room: &mut RoomPresenceState, tracker: &MatrixPresenceTracker) {
    use agent_room_domain::agent_lifecycle::AgentConnection;
    let Some(now) = room
        .instances
        .values()
        .map(ProjectedAgentPresence::observed_at)
        .max()
    else {
        return;
    };
    let disconnected = |instance: &ProjectedAgentPresence| {
        instance
            .evidence_with(tracker.observation(instance.identity().matrix_user_id()))
            .lifecycle(now.value(), room.policy.archive_after_days())
            .connection
            == AgentConnection::Offline
    };
    let mut latest = BTreeMap::new();
    for presence in room.instances.values() {
        if !disconnected(presence) {
            continue;
        }
        let key = presence.identity().agent_id();
        let candidate = (
            presence.published_at(),
            presence.identity().agent_instance_id(),
        );
        latest
            .entry(key)
            .and_modify(|current| {
                if candidate > *current {
                    *current = candidate;
                }
            })
            .or_insert(candidate);
    }
    let keep = room
        .instances
        .iter()
        .filter(|(instance, presence)| {
            !disconnected(presence)
                || latest
                    .get(&presence.identity().agent_id())
                    .is_some_and(|(_, keep)| *instance == keep)
        })
        .map(|(instance, _)| *instance)
        .collect::<BTreeSet<_>>();
    room.instances.retain(|instance, _| keep.contains(instance));
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
    use agent_room_application::ports::{
        MatrixEventId, MatrixRoomId, MatrixUserId, MatrixUserPresence,
    };
    use agent_room_bridge_core::{
        agent_identity::BridgeAgentIdentity,
        presence::{
            PresenceMembershipChange, PresenceProjectionBatch, PresenceQuery,
            PresenceRoomProjection, PresenceRoomProjectionMode, ProjectedAgentPresence,
            ProjectedAgentPresenceFields,
        },
        presence_tracker::MatrixPresenceUpdate,
    };
    use agent_room_domain::{
        agent_lifecycle::{AgentConnection, AgentReception, MatrixPresenceState},
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
    fn 名片按在线状态显示_只问还在房间里又没拿到的名片() {
        let repository = InMemoryPresenceProjectionRepository::default();
        let waiting = card(1, "@waiting:matrix.test");
        let quiet = card(2, "@quiet:matrix.test");
        let gone = card(3, "@gone:matrix.test");
        let batch = PresenceProjectionBatch::new(vec![PresenceRoomProjection::new(
            room_id(),
            PresenceRoomProjectionMode::Replace,
            vec![
                PresenceMembershipChange::new(matrix_user_id(), true),
                joined("@waiting:matrix.test"),
                joined("@quiet:matrix.test"),
                PresenceMembershipChange::new(user("@gone:matrix.test"), false),
            ],
            vec![
                presence("$lease:matrix.test", AgentWorkStatus::Working, 1, 60_000),
                waiting,
                quiet,
                gone,
            ],
        )])
        .with_presence(MatrixPresenceUpdate::new(
            at(1_000),
            true,
            vec![reported(
                "@waiting:matrix.test",
                MatrixPresenceState::Online,
            )],
        ));
        repository.apply_sync(&batch).expect("批次可应用");

        assert_eq!(
            connections(&repository, 1_500),
            [
                (
                    "@agent:matrix.test".to_owned(),
                    AgentConnection::Online,
                    AgentReception::Unknown
                ),
                (
                    "@waiting:matrix.test".to_owned(),
                    AgentConnection::Online,
                    AgentReception::Waiting
                ),
                (
                    "@quiet:matrix.test".to_owned(),
                    AgentConnection::Offline,
                    AgentReception::Unavailable
                ),
            ]
        );
        assert_eq!(
            repository.presence_wanted_sync(at(1_500), 10).unwrap(),
            [user("@quiet:matrix.test")],
            "照租约的、已经拿到的、离开房间的都不问"
        );

        let unavailable = reported("@quiet:matrix.test", MatrixPresenceState::Unavailable);
        repository.state.write().unwrap().presence.record_fetched(
            &user("@quiet:matrix.test"),
            Some(&unavailable),
            at(1_600),
        );
        assert_eq!(
            connections(&repository, 1_700)[2],
            (
                "@quiet:matrix.test".to_owned(),
                AgentConnection::Online,
                AgentReception::OnResume
            )
        );
        assert!(
            repository
                .presence_wanted_sync(at(1_700), 10)
                .unwrap()
                .is_empty()
        );

        // 重新连上：全量同步里只有不离线的人，之前记下的作废，没带的再问一次。
        repository
            .apply_sync(&replace_cards_batch(at(5_000), Vec::new()))
            .expect("批次可应用");
        assert_eq!(
            repository.presence_wanted_sync(at(5_000), 10).unwrap(),
            [user("@quiet:matrix.test"), user("@waiting:matrix.test")]
        );
    }

    fn replace_cards_batch(
        observed_at: UtcMillis,
        online: Vec<MatrixUserPresence>,
    ) -> PresenceProjectionBatch {
        PresenceProjectionBatch::new(vec![PresenceRoomProjection::new(
            room_id(),
            PresenceRoomProjectionMode::Replace,
            vec![joined("@waiting:matrix.test"), joined("@quiet:matrix.test")],
            vec![
                card(1, "@waiting:matrix.test"),
                card(2, "@quiet:matrix.test"),
            ],
        )])
        .with_presence(MatrixPresenceUpdate::new(observed_at, true, online))
    }

    fn connections(
        repository: &InMemoryPresenceProjectionRepository,
        now: i64,
    ) -> Vec<(String, AgentConnection, AgentReception)> {
        repository
            .list_sync(&query(now))
            .expect("Presence 可查询")
            .iter()
            .map(|entry| {
                (
                    entry
                        .presence()
                        .identity()
                        .matrix_user_id()
                        .as_str()
                        .to_owned(),
                    entry.lifecycle.connection,
                    entry.lifecycle.reception,
                )
            })
            .collect()
    }

    /// 写名片的 Agent：ID 和实例号按序号排在固定的 Agent 后面。
    fn card(index: u32, matrix_user_id: &str) -> ProjectedAgentPresence {
        ProjectedAgentPresence::from_verified_fields(ProjectedAgentPresenceFields {
            event_id: MatrixEventId::new(format!("$card-{index}:matrix.test"))
                .expect("事件标识有效"),
            room_id: room_id(),
            identity: BridgeAgentIdentity::new(
                AgentId::from_uuid(
                    Uuid::parse_str(&format!("01945c1e-7b5a-7c7f-8a28-3{index:011x}"))
                        .expect("Agent ID 有效"),
                ),
                format!("Card Agent {index}"),
                matrix_user_id,
                AgentInstanceId::from_uuid(
                    Uuid::parse_str(&format!("01945c1e-7b5a-7c7f-8a28-4{index:011x}"))
                        .expect("实例 ID 有效"),
                ),
            )
            .expect("公开身份有效"),
            status: AgentWorkStatus::Idle,
            observed_at: UtcMillis::new(1_000).expect("观察时间有效"),
            lease_expires_at: UtcMillis::new(301_000).expect("租约时间有效"),
            origin_server_timestamp: 1_000,
            published_at: UtcMillis::new(1_000).expect("发布时间有效"),
            last_polled_at: None,
            listening_until: None,
            reception_known: false,
            card: true,
        })
    }

    fn reported(user_id: &str, state: MatrixPresenceState) -> MatrixUserPresence {
        MatrixUserPresence::new(user(user_id), state, Some(0))
    }

    fn joined(user_id: &str) -> PresenceMembershipChange {
        PresenceMembershipChange::new(user(user_id), true)
    }

    fn user(user_id: &str) -> MatrixUserId {
        MatrixUserId::new(user_id).expect("Matrix 用户标识有效")
    }

    fn at(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("时间有效")
    }

    #[test]
    fn 历次离线实例合并但保留正在连接的会话和同一身份() {
        let repository = InMemoryPresenceProjectionRepository::default();
        let mut presences = (1..=1_000)
            .map(|index| {
                presence_for_instance(
                    &format!("$old-{index}:matrix.test"),
                    AgentWorkStatus::Offline,
                    index,
                    2_000,
                    &format!("01945c1e-7b5a-7c7f-8a28-{index:012x}"),
                )
            })
            .collect::<Vec<_>>();
        presences.push(presence(
            "$current:matrix.test",
            AgentWorkStatus::Idle,
            1_001,
            60_000,
        ));
        repository
            .apply_sync(&PresenceProjectionBatch::new(vec![
                PresenceRoomProjection::new(
                    room_id(),
                    PresenceRoomProjectionMode::Replace,
                    vec![PresenceMembershipChange::new(matrix_user_id(), true)],
                    presences,
                ),
            ]))
            .expect("批次可应用");
        assert_eq!(
            repository.state.read().expect("状态锁").rooms[&room_id()]
                .instances
                .len(),
            2
        );
        let observations = repository.list_sync(&query(1_500)).expect("可查询");
        assert_eq!(observations.len(), 1);
        assert_eq!(
            observations[0].presence().event_id().as_str(),
            "$current:matrix.test"
        );
        assert_eq!(
            observations[0].lifecycle.connection,
            AgentConnection::Online
        );
        assert_eq!(observations[0].lifecycle.archive_reason, None);
    }

    #[test]
    fn 归档策略增量同步并在完整状态缺失时恢复默认值() {
        let repository = InMemoryPresenceProjectionRepository::default();
        repository
            .apply_sync(&replace_batch(presence(
                "$offline:matrix.test",
                AgentWorkStatus::Offline,
                10,
                2_000,
            )))
            .expect("初始状态");
        repository
            .apply_sync(&PresenceProjectionBatch::new(vec![
                PresenceRoomProjection::new(
                    room_id(),
                    PresenceRoomProjectionMode::Delta,
                    Vec::new(),
                    Vec::new(),
                )
                .with_policy(
                    agent_room_domain::agent_lifecycle::AgentRosterPolicy::new(30),
                ),
            ]))
            .expect("共享规则");
        let now = 8 * 86_400_000;
        assert_eq!(
            repository.list_sync(&query(now)).expect("可查询")[0]
                .lifecycle
                .archive_reason,
            None
        );
        repository
            .apply_sync(&replace_batch(presence(
                "$offline:matrix.test",
                AgentWorkStatus::Offline,
                10,
                2_000,
            )))
            .expect("完整状态");
        assert_eq!(
            repository.list_sync(&query(now)).expect("可查询")[0]
                .lifecycle
                .archive_reason,
            Some(agent_room_domain::agent_lifecycle::AgentArchiveReason::Expired)
        );
    }

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
        presence_for_instance(
            event_id,
            status,
            origin_server_timestamp,
            lease_expires_at,
            INSTANCE_ID,
        )
    }

    fn presence_for_instance(
        event_id: &str,
        status: AgentWorkStatus,
        origin_server_timestamp: u64,
        lease_expires_at: i64,
        instance_id: &str,
    ) -> ProjectedAgentPresence {
        ProjectedAgentPresence::from_verified_fields(ProjectedAgentPresenceFields {
            event_id: MatrixEventId::new(event_id).expect("事件标识有效"),
            room_id: room_id(),
            identity: BridgeAgentIdentity::new(
                AgentId::from_uuid(Uuid::parse_str(AGENT_ID).expect("Agent ID 有效")),
                "Presence Agent",
                MATRIX_USER_ID,
                AgentInstanceId::from_uuid(Uuid::parse_str(instance_id).expect("实例 ID 有效")),
            )
            .expect("公开身份有效"),
            status,
            observed_at: UtcMillis::new(1_000).expect("观察时间有效"),
            lease_expires_at: UtcMillis::new(lease_expires_at).expect("租约时间有效"),
            origin_server_timestamp,
            published_at: UtcMillis::new(1_000).expect("发布时间有效"),
            last_polled_at: None,
            listening_until: None,
            reception_known: false,
            card: false,
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
