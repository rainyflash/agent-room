//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：本机 Agent 照主人给的管理授权
//! 禁言、踢出、封禁、解封。落进的是同一套台账，执行人是给授权的人，另记是哪个 Agent 做的；动别人的 Agent 等于
//! 动它的主人；房主、管理员、给授权的人和他们的 Agent 动不了。

#[path = "agents/kick_ban.rs"]
mod kick_ban;

use agent_room_application::{
    devices::AuthenticatedDevice,
    moderation::{
        AgentModerate, AgentModerationDependencies, AgentModerationFailure,
        AgentModerationFailureKind, AgentModerationScope, AgentModerationService,
        AgentModerationUseCases, ListAgentModeration,
    },
    ports::{
        AgentLobbyAccessRecord, AgentLobbyAccessRepository, AgentModerationAttribution,
        AgentModerationRecord, AgentModerationReversal, AgentModerationStore, AppliedBan,
        ModerationSubject, PrincipalAccount, PrivateRoomSnapshot, PrivateRoomStore,
        RoomModerationGrant,
    },
};
use agent_room_domain::{
    identity::Principal,
    ids::{AgentId, AgentInstanceId, DeviceId, RoomInstanceId, RoomModerationGrantId},
    private_rooms::{PrivateRoom, PrivateRoomPermissions},
    room_moderation_grants::{
        AgentModerationNote, RoomModerationPower, RoomModerationPowers, RoomModerationRules,
    },
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogStatus,
        RoomCatalogVisibility, RoomInstance, RoomInstanceFields, RoomInstanceState,
    },
    version::AggregateVersion,
};

use super::*;

/// 记着授权、认人表、Agent 做的和撤回的那几条；预留动作、记成撤回照台账的假仓库记。
struct FakeAgentStore {
    repository: Arc<FakeRepository>,
    grant: Mutex<Option<RoomModerationGrant>>,
    subjects: HashMap<String, ModerationSubject>,
    attributions: Mutex<Vec<AgentModerationAttribution>>,
    reversals: Mutex<Vec<AgentModerationReversal>>,
}

fn record(attribution: &AgentModerationAttribution) -> AgentModerationRecord {
    AgentModerationRecord {
        attribution: attribution.clone(),
        agent_display_name: "门卫 #2316".to_owned(),
    }
}

impl AgentModerationStore for FakeAgentStore {
    fn active_agent_grant(
        &self,
        room_catalog_id: RoomCatalogId,
        agent_id: AgentId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomModerationGrant>>> {
        let grant = self
            .grant
            .lock()
            .expect("授权锁可用")
            .clone()
            .filter(|grant| {
                grant.catalog_id == room_catalog_id
                    && grant.agent_id == agent_id
                    && grant.is_active_at(now)
            });
        Box::pin(async move { Ok(grant) })
    }

    fn moderation_subject<'a>(
        &'a self,
        matrix_user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationSubject>>> {
        let subject = self.subjects.get(matrix_user_id.as_str()).copied();
        Box::pin(async move { Ok(subject) })
    }

    /// 和真实仓库一样：同一个动作已经在了就交回它、不再记；不然预留并记下是哪个 Agent 做的。
    fn reserve_agent_action<'a>(
        &'a self,
        action: &'a ModerationAction,
        audit: &'a ModerationAuditEvent,
        attribution: &'a AgentModerationAttribution,
    ) -> PortFuture<'a, RepositoryResult<ModerationActionReservationOutcome>> {
        let existing = self
            .repository
            .actions
            .lock()
            .expect("动作锁可用")
            .iter()
            .find(|stored| stored.id() == action.id())
            .cloned();
        let outcome = if let Some(existing) = existing {
            ModerationActionReservationOutcome::Existing(existing)
        } else {
            self.repository
                .actions
                .lock()
                .expect("动作锁可用")
                .push(action.clone());
            self.repository
                .audits
                .lock()
                .expect("审计锁可用")
                .push(audit.clone());
            self.attributions
                .lock()
                .expect("记账锁可用")
                .push(attribution.clone());
            ModerationActionReservationOutcome::Reserved(action.clone())
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn agent_actions(
        &self,
        _room_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentModerationRecord>>> {
        let records = self
            .attributions
            .lock()
            .expect("记账锁可用")
            .iter()
            .map(record)
            .collect();
        Box::pin(async move { Ok(records) })
    }

    fn applied_bans(
        &self,
        _room_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AppliedBan>>> {
        let attributions = self.attributions.lock().expect("记账锁可用").clone();
        let bans = self
            .repository
            .actions
            .lock()
            .expect("动作锁可用")
            .iter()
            .filter(|action| {
                action.kind() == ModerationActionKind::Ban
                    && action.status() == ModerationActionStatus::Applied
            })
            .map(|action| AppliedBan {
                action: action.clone(),
                by_agent: attributions
                    .iter()
                    .any(|attribution| attribution.action_id == action.id()),
            })
            .collect();
        Box::pin(async move { Ok(bans) })
    }

    /// 照台账的假仓库记成撤回；成了再记下是哪个 Agent 撤的。
    fn finalize_agent_reversal<'a>(
        &'a self,
        action: &'a ModerationAction,
        audit: &'a ModerationAuditEvent,
        reversal: &'a AgentModerationReversal,
    ) -> PortFuture<'a, RepositoryResult<ModerationAction>> {
        Box::pin(async move {
            let reversed = self.repository.finalize_action(action, audit).await?;
            self.reversals
                .lock()
                .expect("撤回锁可用")
                .push(reversal.clone());
            Ok(reversed)
        })
    }

    fn agent_reversal(
        &self,
        request_id: ModerationActionId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentModerationReversal>>> {
        let found = self
            .reversals
            .lock()
            .expect("撤回锁可用")
            .iter()
            .rev()
            .find(|reversal| reversal.request_id == request_id)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn agent_reversals(
        &self,
        _room_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentModerationRecord>>> {
        let records = self
            .reversals
            .lock()
            .expect("撤回锁可用")
            .iter()
            .map(|reversal| record(&reversal.attribution))
            .collect();
        Box::pin(async move { Ok(records) })
    }
}

struct FakeAccess(Mutex<AgentLobbyAccessRecord>);

impl AgentLobbyAccessRepository for FakeAccess {
    fn find_lobby_access(
        &self,
        agent_instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentLobbyAccessRecord>>> {
        let access = self.0.lock().expect("连接锁可用").clone();
        let found = (access.agent_instance_id == agent_instance_id).then_some(access);
        Box::pin(async move { Ok(found) })
    }

    fn lobby_accesses_of_agent(
        &self,
        _agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentLobbyAccessRecord>>> {
        Box::pin(async { unreachable!("Agent 管房间不列连接") })
    }

    fn find_public_lobby_room<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<RoomInstanceId>>> {
        Box::pin(async { unreachable!("Agent 管房间不找大厅分片") })
    }
}

struct FakeRooms(Mutex<PrivateRoomSnapshot>);

impl PrivateRoomStore for FakeRooms {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("Agent 管房间不建房间") })
    }

    fn find_by_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        let snapshot = self.0.lock().expect("房间锁可用").clone();
        let found = (catalog_id == snapshot.catalog().id()).then_some(snapshot);
        Box::pin(async move { Ok(found) })
    }

    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("Agent 管房间不列房间") })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("Agent 管房间按目录找房间") })
    }

    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("Agent 管房间不改成员") })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("Agent 管房间不改名") })
    }
}

struct AgentFixture {
    ledger: Fixture,
    store: Arc<FakeAgentStore>,
    access: Arc<FakeAccess>,
    rooms: Arc<FakeRooms>,
    service: AgentModerationService,
}

impl AgentFixture {
    /// 房主（1）把房间交给管理员（4）的 Agent 管，授权里只勾了禁言；它的连接在管理员（4）的电脑上。
    fn new() -> Self {
        let ledger = Fixture::new();
        let store = Arc::new(FakeAgentStore {
            repository: ledger.repository.clone(),
            grant: Mutex::new(Some(grant(&[RoomModerationPower::Mute]))),
            subjects: subjects(),
            attributions: Mutex::new(Vec::new()),
            reversals: Mutex::new(Vec::new()),
        });
        let access = Arc::new(FakeAccess(Mutex::new(AgentLobbyAccessRecord {
            agent_id: agent_id(),
            agent_instance_id: instance_id(),
            device_id: device_id(),
            principal_id: granter(),
            matrix_user_id: MatrixUserId::new("@doorkeeper:matrix.test").expect("Agent 有效"),
            active: true,
        })));
        let rooms = Arc::new(FakeRooms(Mutex::new(snapshot())));
        let service = AgentModerationService::new(AgentModerationDependencies {
            moderation: ledger.service.clone(),
            store: store.clone(),
            access: access.clone(),
            rooms: rooms.clone(),
        });
        Self {
            ledger,
            store,
            access,
            rooms,
            service,
        }
    }

    fn grant(&self, powers: &[RoomModerationPower]) {
        *self.store.grant.lock().expect("授权锁可用") = Some(grant(powers));
    }

    async fn mute(&self, target: &str) -> Result<ModerationAction, AgentModerationFailureKind> {
        self.service
            .moderate(request(target))
            .await
            .map_err(AgentModerationFailure::kind)
    }
}

#[tokio::test]
async fn agent_照授权禁言_执行人是给授权的人_另记是它做的() {
    let fixture = AgentFixture::new();
    let request = request("@spammer:matrix.test");

    let action = fixture
        .service
        .moderate(request.clone())
        .await
        .expect("授权里有禁言");

    assert_eq!(action.id(), request.action_id);
    assert_eq!(action.status(), ModerationActionStatus::Applied);
    assert_eq!(action.actor_principal_id(), granter());
    assert_eq!(action.kind(), ModerationActionKind::Mute);
    assert_eq!(action.target().reference(), spammer().to_string());
    assert_eq!(action.expires_at(), Some(time(NOW + 600_000)));
    assert_eq!(
        fixture.ledger.effects.applied_rooms(),
        ["!room:matrix.test"]
    );
    assert!(all_carry_agents(&fixture.ledger.effects.applied));
    assert_eq!(
        *fixture.store.attributions.lock().unwrap(),
        [AgentModerationAttribution {
            action_id: request.action_id,
            agent_id: agent_id(),
            grant_id: grant_id(),
            note: request.note.clone(),
        }]
    );

    // 同一个动作 ID 再来一次交回记下的那条，不再落一遍。
    let replayed = fixture.service.moderate(request).await.expect("重放");
    assert_eq!(replayed.id(), action.id());
    assert_eq!(fixture.ledger.effects.applied_rooms().len(), 1);
    assert_eq!(fixture.store.attributions.lock().unwrap().len(), 1);

    let records = fixture
        .service
        .list(ListAgentModeration {
            actor: actor(false),
            room_catalog_id: room_id(),
        })
        .await
        .expect("房主和管理员看得到");
    assert_eq!(records.actions[0].attribution.action_id, action.id());
    assert_eq!(
        records.actions[0].attribution.note.as_str(),
        "连发了五条广告链接。"
    );
    assert!(records.reversals.is_empty());
}

#[tokio::test]
async fn 动别人的_agent_等于动它的主人_网络_agent_只动它自己() {
    let fixture = AgentFixture::new();

    let action = fixture
        .mute("@spammer-agent:matrix.test")
        .await
        .expect("认成它的主人");
    assert_eq!(action.target().reference(), spammer().to_string());

    let network = fixture
        .mute("@network-agent:matrix.test")
        .await
        .expect("网络 Agent 认成替它记账的合成主体");
    assert_eq!(network.target().reference(), Uuid::from_u128(9).to_string());
    assert_eq!(
        fixture.mute("@nobody:matrix.test").await.unwrap_err(),
        AgentModerationFailureKind::TargetNotFound
    );
}

#[tokio::test]
async fn 房主_管理员_给授权的人和他们的_agent_都动不了() {
    let fixture = AgentFixture::new();
    for target in [
        "@owner:matrix.test",
        "@owner-agent:matrix.test",
        "@admin:matrix.test",
        "@invited-admin:matrix.test",
        "@granter:matrix.test",
        "@granter-agent:matrix.test",
    ] {
        assert_eq!(
            fixture.mute(target).await.unwrap_err(),
            AgentModerationFailureKind::Protected,
            "{target}"
        );
    }
    assert!(fixture.ledger.repository.actions.lock().unwrap().is_empty());
    assert!(fixture.ledger.effects.applied_rooms().is_empty());
}

#[tokio::test]
async fn 授权里没有这一样_或者时长和理由不对时不动手() {
    let fixture = AgentFixture::new();
    let kick = AgentModerate {
        power: RoomModerationPower::Kick,
        duration: None,
        ..request("@spammer:matrix.test")
    };
    assert_eq!(
        fixture
            .service
            .moderate(kick.clone())
            .await
            .unwrap_err()
            .kind(),
        AgentModerationFailureKind::NotAllowed
    );

    fixture.grant(&[RoomModerationPower::Mute, RoomModerationPower::Kick]);
    for invalid in [
        // 只有禁言收时长。
        AgentModerate {
            duration: Some(DurationMillis::new(600_000).unwrap()),
            ..kick.clone()
        },
        AgentModerate {
            reason: None,
            ..kick
        },
    ] {
        assert_eq!(
            fixture.service.moderate(invalid).await.unwrap_err().kind(),
            AgentModerationFailureKind::InvalidRequest
        );
    }

    let too_short = AgentModerate {
        duration: Some(DurationMillis::new(30_000).unwrap()),
        ..request("@spammer:matrix.test")
    };
    assert_eq!(
        fixture
            .service
            .moderate(too_short)
            .await
            .unwrap_err()
            .kind(),
        AgentModerationFailureKind::InvalidRequest
    );
    assert!(fixture.ledger.repository.actions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 连接对不上_授权撤了到期了_或者给授权的人不再管房间时不动手() {
    let fixture = AgentFixture::new();
    fixture.access.0.lock().unwrap().active = false;
    assert_eq!(
        fixture.mute("@spammer:matrix.test").await.unwrap_err(),
        AgentModerationFailureKind::Unauthorized
    );
    fixture.access.0.lock().unwrap().active = true;
    fixture.access.0.lock().unwrap().agent_id = AgentId::from_uuid(Uuid::from_u128(99));
    assert_eq!(
        fixture.mute("@spammer:matrix.test").await.unwrap_err(),
        AgentModerationFailureKind::Unauthorized
    );
    fixture.access.0.lock().unwrap().agent_id = agent_id();

    fixture.ledger.runtime.advance(3_600_001);
    assert_eq!(
        fixture.mute("@spammer:matrix.test").await.unwrap_err(),
        AgentModerationFailureKind::NotGranted
    );

    *fixture.store.grant.lock().unwrap() = Some(RoomModerationGrant {
        expires_at: time(NOW + 7_200_000),
        ..grant(&[RoomModerationPower::Mute])
    });
    *fixture.ledger.authority.room_role.lock().unwrap() = ModerationRole::None;
    assert_eq!(
        fixture.mute("@spammer:matrix.test").await.unwrap_err(),
        AgentModerationFailureKind::NotGranted
    );
    assert!(fixture.ledger.repository.actions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 不管房间的人看不到哪些是_agent_做的() {
    let fixture = AgentFixture::new();
    *fixture.ledger.authority.room_role.lock().unwrap() = ModerationRole::None;

    let failure = fixture
        .service
        .list(ListAgentModeration {
            actor: actor(false),
            room_catalog_id: room_id(),
        })
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), AgentModerationFailureKind::Forbidden);
}

fn request(target: &str) -> AgentModerate {
    AgentModerate {
        scope: AgentModerationScope {
            device: device(),
            agent_id: agent_id(),
            agent_instance_id: instance_id(),
            room_catalog_id: room_id(),
        },
        action_id: ModerationActionId::from_uuid(Uuid::now_v7()),
        power: RoomModerationPower::Mute,
        target: MatrixUserId::new(target).expect("测试账号有效"),
        reason: Some(ModerationReason::Spam),
        note: AgentModerationNote::parse("连发了五条广告链接。").expect("理由有效"),
        duration: Some(DurationMillis::new(600_000).expect("十分钟有效")),
    }
}

/// 一个小时以后到期的授权，房主（1）给的是管理员（4）的 Agent。
fn grant(powers: &[RoomModerationPower]) -> RoomModerationGrant {
    RoomModerationGrant {
        id: grant_id(),
        catalog_id: room_id(),
        agent_id: agent_id(),
        granted_by: granter(),
        rules: RoomModerationRules::parse("发广告的禁言十分钟。").expect("规则有效"),
        powers: RoomModerationPowers::new(powers.iter().copied()).expect("能做的几样有效"),
        created_at: time(NOW - 1_000),
        expires_at: time(NOW + 3_600_000),
        revoked_at: None,
    }
}

/// 认人表：人的账号是他，Agent 的账号是它的主人。
fn subjects() -> HashMap<String, ModerationSubject> {
    let person = |principal_id| ModerationSubject {
        principal_id,
        network_agent: false,
    };
    HashMap::from([
        ("@spammer:matrix.test".to_owned(), person(spammer())),
        ("@spammer-agent:matrix.test".to_owned(), person(spammer())),
        ("@owner:matrix.test".to_owned(), person(principal_id())),
        (
            "@owner-agent:matrix.test".to_owned(),
            person(principal_id()),
        ),
        ("@admin:matrix.test".to_owned(), person(admin())),
        (
            "@invited-admin:matrix.test".to_owned(),
            person(invited_admin()),
        ),
        ("@granter:matrix.test".to_owned(), person(granter())),
        ("@granter-agent:matrix.test".to_owned(), person(granter())),
        (
            "@network-agent:matrix.test".to_owned(),
            ModerationSubject {
                principal_id: PrincipalId::from_uuid(Uuid::from_u128(9)),
                network_agent: true,
            },
        ),
    ])
}

/// 房主是台账测试里的执行人（1）；管理员（5）已加入、另一位（6）还在受邀；给授权的是管理员（4）；被禁言的是
/// 一位普通成员（2）。
fn snapshot() -> PrivateRoomSnapshot {
    let mut room = PrivateRoom::create(room_id(), principal_id());
    for (member, permissions, accept) in [
        (granter(), PrivateRoomPermissions::ALL, true),
        (admin(), PrivateRoomPermissions::ALL, true),
        (invited_admin(), PrivateRoomPermissions::ALL, false),
        (spammer(), PrivateRoomPermissions::INVITED_BY_AGENT, true),
    ] {
        room.invite(principal_id(), member, permissions)
            .expect("邀请有效");
        if accept {
            room.accept_invitation(member).expect("接受邀请");
        }
    }
    let catalog = RoomCatalog::new(
        room_id(),
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: "读书会".to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(principal_id()),
            visibility: RoomCatalogVisibility::Private,
            retention_days: None,
            status: RoomCatalogStatus::Active,
        },
    )
    .expect("目录有效");
    let instance = RoomInstance::restore(
        RoomInstanceId::from_uuid(Uuid::from_u128(20)),
        RoomInstanceFields {
            catalog_id: room_id(),
            matrix_room_id: MatrixRoomReference::new("!room:matrix.test".to_owned())
                .expect("房间有效"),
            region: None,
            capacity: RoomCapacity::standard(),
            projected_member_count: 0,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .expect("分片有效");
    PrivateRoomSnapshot::new(catalog, instance, room).expect("快照有效")
}

fn device() -> AuthenticatedDevice {
    AuthenticatedDevice {
        account: PrincipalAccount {
            principal: Principal::new(granter()),
            matrix_user_id: "@granter:matrix.test".to_owned(),
            display_name: "管理员".to_owned(),
            avatar_content_id: None,
            locale: "zh-CN".to_owned(),
        },
        device_id: device_id(),
        access_token_expires_at: time(NOW + 86_400_000),
    }
}

fn spammer() -> PrincipalId {
    target_id()
}

fn granter() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(4))
}

fn admin() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(5))
}

fn invited_admin() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(6))
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(Uuid::from_u128(31))
}

fn instance_id() -> AgentInstanceId {
    AgentInstanceId::from_uuid(Uuid::from_u128(41))
}

fn device_id() -> DeviceId {
    DeviceId::from_uuid(Uuid::from_u128(51))
}

fn grant_id() -> RoomModerationGrantId {
    RoomModerationGrantId::from_uuid(Uuid::from_u128(61))
}
