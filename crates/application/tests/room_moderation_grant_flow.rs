//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：房主和管理员给自己的
//! Agent 管理授权，要最近登录过；同一个 Agent 再给一次盖掉上一份；到期了的不列；给的人自己总能撤，
//! 别人要是此刻的房主或管理员。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        ActiveRoomModerationGrant, Clock, PortFuture, PrivateRoomSnapshot, PrivateRoomStore,
        RoomModerationGrant, RoomModerationGrantStore,
    },
    private_rooms::{
        GrantRoomModeration, ListRoomModerationGrants, ModerationGrantFailureKind,
        RevokeRoomModerationGrant, RoomModerationGrantDependencies, RoomModerationGrantService,
        RoomModerationGrantUseCases,
    },
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId, RoomInstanceId, RoomModerationGrantId},
    private_rooms::{PrivateRoom, PrivateRoomPermissions},
    room_moderation_grants::{MAX_ROOM_MODERATION_GRANT_LIFETIME_MILLIS, RoomModerationPower},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
    },
    time::{DurationMillis, UtcMillis},
    version::AggregateVersion,
};
use uuid::Uuid;

const NOW: i64 = 1_790_000_000_000;

#[tokio::test]
async fn 房主和管理员给自己的_agent_再给一次盖掉上一份() {
    let fixture = Fixture::new(room());
    let first = fixture
        .service
        .grant(grant(owner(), owner_agent(), 1))
        .await
        .expect("房主能给自己的 Agent");
    assert_eq!(first.agent_display_name, "看门的 #2316");
    assert_eq!(first.grant.granted_by, owner());
    assert_eq!(first.grant.rules.as_str(), "说 spam 的禁言 10 分钟。");
    assert_eq!(
        first.grant.powers.as_slice(),
        [RoomModerationPower::Mute, RoomModerationPower::Ban]
    );
    assert_eq!(
        first.grant.expires_at,
        time(NOW + i64::try_from(MAX_ROOM_MODERATION_GRANT_LIFETIME_MILLIS).unwrap())
    );
    fixture
        .service
        .grant(GrantRoomModeration {
            lifetime: Some(DurationMillis::new(3_600_000).unwrap()),
            ..grant(owner(), owner_agent(), 2)
        })
        .await
        .expect("再给一次");
    fixture
        .service
        .grant(grant(manager(), manager_agent(), 3))
        .await
        .expect("管理员也能给");

    let listed = fixture.service.list(list(manager())).await.unwrap();
    let ids: Vec<_> = listed.iter().map(|active| active.grant.id).collect();
    assert_eq!(ids, [grant_id(2), grant_id(3)], "第一份被盖掉了");
    assert_eq!(listed[0].grant.expires_at, time(NOW + 3_600_000));
    assert_eq!(fixture.revoked_by(grant_id(1)), Some(owner()));
}

#[tokio::test]
async fn 普通成员_路人_别人的_agent_没最近登录_写错的都不给() {
    let fixture = Fixture::new(room());
    for (request, expected) in [
        (
            grant(member(), member_agent(), 1),
            ModerationGrantFailureKind::Forbidden,
        ),
        (
            grant(stranger(), owner_agent(), 1),
            ModerationGrantFailureKind::NotFound,
        ),
        (
            grant(owner(), manager_agent(), 1),
            ModerationGrantFailureKind::NotYourAgent,
        ),
        (
            GrantRoomModeration {
                actor: principal(owner(), false),
                ..grant(owner(), owner_agent(), 1)
            },
            ModerationGrantFailureKind::Forbidden,
        ),
        (
            GrantRoomModeration {
                rules: " \n ".to_owned(),
                ..grant(owner(), owner_agent(), 1)
            },
            ModerationGrantFailureKind::InvalidRequest,
        ),
        (
            GrantRoomModeration {
                powers: Vec::new(),
                ..grant(owner(), owner_agent(), 1)
            },
            ModerationGrantFailureKind::InvalidRequest,
        ),
        (
            GrantRoomModeration {
                lifetime: Some(DurationMillis::new(1_000).unwrap()),
                ..grant(owner(), owner_agent(), 1)
            },
            ModerationGrantFailureKind::InvalidRequest,
        ),
    ] {
        let actor = request.actor.principal_id;
        let failure = fixture.service.grant(request).await.unwrap_err();
        assert_eq!(failure.kind(), expected, "{actor:?}");
    }
    assert!(fixture.grants.lock().unwrap().is_empty());

    let failure = fixture.service.list(list(member())).await.unwrap_err();
    assert_eq!(failure.kind(), ModerationGrantFailureKind::Forbidden);

    let mut archived = room();
    archived.archive(owner()).unwrap();
    let failure = Fixture::new(archived)
        .service
        .grant(grant(owner(), owner_agent(), 1))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), ModerationGrantFailureKind::Conflict);
}

#[tokio::test]
async fn 到期了的不列() {
    let fixture = Fixture::new(room());
    fixture
        .service
        .grant(grant(owner(), owner_agent(), 1))
        .await
        .unwrap();
    fixture.grants.lock().unwrap()[0].expires_at = time(NOW);
    assert!(
        fixture
            .service
            .list(list(owner()))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn 给的人不当管理员了也能撤_房主能撤别人的_普通成员不能() {
    let mut demoted = room();
    let fixture = Fixture::new(room());
    fixture
        .service
        .grant(grant(manager(), manager_agent(), 1))
        .await
        .unwrap();
    demoted
        .update_permissions(owner(), manager(), PrivateRoomPermissions::INVITED_BY_AGENT)
        .unwrap();
    *fixture.room.lock().unwrap() = demoted;

    let failure = fixture
        .service
        .revoke(revoke(member(), 1))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), ModerationGrantFailureKind::Forbidden);
    let failure = fixture
        .service
        .revoke(RevokeRoomModerationGrant {
            catalog_id: RoomCatalogId::from_uuid(Uuid::from_u128(99)),
            ..revoke(manager(), 1)
        })
        .await
        .unwrap_err();
    assert_eq!(
        failure.kind(),
        ModerationGrantFailureKind::NotFound,
        "别的房间的授权当作没有"
    );
    fixture
        .service
        .revoke(revoke(manager(), 1))
        .await
        .expect("给的人自己能撤");
    assert_eq!(fixture.revoked_by(grant_id(1)), Some(manager()));
    fixture
        .service
        .revoke(revoke(owner(), 1))
        .await
        .expect("已经撤了的也算撤了");
    assert_eq!(fixture.revoked_by(grant_id(1)), Some(manager()));

    fixture
        .service
        .grant(grant(owner(), owner_agent(), 2))
        .await
        .unwrap();
    let failure = fixture
        .service
        .revoke(revoke(manager(), 2))
        .await
        .unwrap_err();
    assert_eq!(
        failure.kind(),
        ModerationGrantFailureKind::Forbidden,
        "不当管理员了，撤不了别人给的"
    );
}

struct Fixture {
    service: RoomModerationGrantService,
    room: Arc<Mutex<PrivateRoom>>,
    grants: Arc<Mutex<Vec<RoomModerationGrant>>>,
    revokers: Arc<Mutex<Vec<(RoomModerationGrantId, PrincipalId)>>>,
}

impl Fixture {
    fn new(room: PrivateRoom) -> Self {
        let room = Arc::new(Mutex::new(room));
        let grants = Arc::new(Mutex::new(Vec::new()));
        let revokers = Arc::new(Mutex::new(Vec::new()));
        let service = RoomModerationGrantService::new(RoomModerationGrantDependencies {
            rooms: Arc::new(MemoryRooms(room.clone())),
            grants: Arc::new(MemoryGrants {
                grants: grants.clone(),
                revokers: revokers.clone(),
            }),
            clock: Arc::new(TestClock),
        });
        Self {
            service,
            room,
            grants,
            revokers,
        }
    }

    /// 谁撤的这一份；没撤的为 `None`。
    fn revoked_by(&self, id: RoomModerationGrantId) -> Option<PrincipalId> {
        self.revokers
            .lock()
            .unwrap()
            .iter()
            .find(|(grant, _)| *grant == id)
            .map(|(_, by)| *by)
    }
}

fn grant(actor: PrincipalId, agent: AgentId, id: u128) -> GrantRoomModeration {
    GrantRoomModeration {
        actor: principal(actor, true),
        catalog_id: catalog_id(),
        grant_id: grant_id(id),
        agent_id: agent,
        rules: "  说 spam 的禁言 10 分钟。 ".to_owned(),
        powers: vec![
            RoomModerationPower::Ban,
            RoomModerationPower::Mute,
            RoomModerationPower::Ban,
        ],
        lifetime: None,
    }
}

fn list(actor: PrincipalId) -> ListRoomModerationGrants {
    ListRoomModerationGrants {
        actor: principal(actor, false),
        catalog_id: catalog_id(),
    }
}

fn revoke(actor: PrincipalId, id: u128) -> RevokeRoomModerationGrant {
    RevokeRoomModerationGrant {
        actor: principal(actor, false),
        catalog_id: catalog_id(),
        grant_id: grant_id(id),
    }
}

/// 房主、一位管理员、一位能说话不能管理的成员。
fn room() -> PrivateRoom {
    let mut room = PrivateRoom::create(catalog_id(), owner());
    room.invite(owner(), manager(), PrivateRoomPermissions::ALL)
        .unwrap();
    room.accept_invitation(manager()).unwrap();
    room.invite(owner(), member(), PrivateRoomPermissions::INVITED_BY_AGENT)
        .unwrap();
    room.accept_invitation(member()).unwrap();
    room
}

fn snapshot(room: PrivateRoom) -> PrivateRoomSnapshot {
    let archived = room.status().as_str() == "archived";
    let catalog = RoomCatalog::new(
        catalog_id(),
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: "读书会".to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(owner()),
            visibility: RoomCatalogVisibility::Private,
            retention_days: None,
            status: if archived {
                RoomCatalogStatus::Archived
            } else {
                RoomCatalogStatus::Active
            },
        },
    )
    .unwrap();
    let instance = RoomInstance::restore(
        RoomInstanceId::from_uuid(Uuid::from_u128(20)),
        RoomInstanceFields {
            catalog_id: catalog_id(),
            matrix_room_id: MatrixRoomReference::new("!club:matrix.test".to_owned()).unwrap(),
            region: None,
            capacity: RoomCapacity::standard(),
            projected_member_count: 0,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: if archived {
                RoomInstanceState::Archived
            } else {
                RoomInstanceState::Active
            },
        },
    )
    .unwrap();
    PrivateRoomSnapshot::new(catalog, instance, room).unwrap()
}

fn principal(principal_id: PrincipalId, recently_authenticated: bool) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id,
        matrix_user_id: format!("@user-{}:matrix.test", principal_id.as_uuid().as_u128()),
        display_name: "Someone".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: time(NOW + 60_000),
        recently_authenticated,
    }
}

fn owner() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(1))
}

fn manager() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(2))
}

fn member() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(3))
}

fn stranger() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(4))
}

fn owner_agent() -> AgentId {
    AgentId::from_uuid(Uuid::from_u128(31))
}

fn manager_agent() -> AgentId {
    AgentId::from_uuid(Uuid::from_u128(32))
}

fn member_agent() -> AgentId {
    AgentId::from_uuid(Uuid::from_u128(33))
}

fn grant_id(value: u128) -> RoomModerationGrantId {
    RoomModerationGrantId::from_uuid(Uuid::from_u128(100 + value))
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(10))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}

struct TestClock;

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

/// 每个人的 Agent 只认它的主人；撤的时候记下是谁撤的。
struct MemoryGrants {
    grants: Arc<Mutex<Vec<RoomModerationGrant>>>,
    revokers: Arc<Mutex<Vec<(RoomModerationGrantId, PrincipalId)>>>,
}

impl RoomModerationGrantStore for MemoryGrants {
    fn owned_agent_name(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<String>>> {
        let name = [
            (owner(), owner_agent(), "看门的 #2316"),
            (manager(), manager_agent(), "管理员的 #0001"),
            (member(), member_agent(), "成员的 #0002"),
        ]
        .into_iter()
        .find(|(owner, agent, _)| *owner == principal_id && *agent == agent_id)
        .map(|(_, _, name)| name.to_owned());
        Box::pin(async move { Ok(name) })
    }

    fn replace_room_moderation_grant<'a>(
        &'a self,
        grant: &'a RoomModerationGrant,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        let mut grants = self.grants.lock().unwrap();
        for existing in grants.iter_mut().filter(|existing| {
            existing.catalog_id == grant.catalog_id
                && existing.agent_id == grant.agent_id
                && existing.revoked_at.is_none()
        }) {
            existing.revoked_at = Some(grant.created_at);
            self.revokers
                .lock()
                .unwrap()
                .push((existing.id, grant.granted_by));
        }
        grants.push(grant.clone());
        Box::pin(async { Ok(()) })
    }

    fn active_room_moderation_grants(
        &self,
        catalog_id: RoomCatalogId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<ActiveRoomModerationGrant>>> {
        let active = self
            .grants
            .lock()
            .unwrap()
            .iter()
            .filter(|grant| grant.catalog_id == catalog_id && grant.is_active_at(now))
            .map(|grant| ActiveRoomModerationGrant {
                grant: grant.clone(),
                agent_display_name: "名字".to_owned(),
            })
            .collect();
        Box::pin(async move { Ok(active) })
    }

    fn room_moderation_grant(
        &self,
        id: RoomModerationGrantId,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomModerationGrant>>> {
        let found = self
            .grants
            .lock()
            .unwrap()
            .iter()
            .find(|grant| grant.id == id)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn revoke_room_moderation_grant(
        &self,
        id: RoomModerationGrantId,
        revoked_by: PrincipalId,
        revoked_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        let mut grants = self.grants.lock().unwrap();
        let revoked = grants
            .iter_mut()
            .find(|grant| grant.id == id && grant.revoked_at.is_none())
            .map(|grant| grant.revoked_at = Some(revoked_at))
            .is_some();
        if revoked {
            self.revokers.lock().unwrap().push((id, revoked_by));
        }
        Box::pin(async move { Ok(revoked) })
    }
}

struct MemoryRooms(Arc<Mutex<PrivateRoom>>);

impl PrivateRoomStore for MemoryRooms {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("管理授权不建房间") })
    }

    fn find_by_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        let room = self.0.lock().unwrap().clone();
        let found = (catalog_id == room.catalog_id()).then(|| snapshot(room));
        Box::pin(async move { Ok(found) })
    }

    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("管理授权不列房间") })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("管理授权按目录找房间") })
    }

    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("管理授权不改成员") })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("管理授权不改名") })
    }
}
