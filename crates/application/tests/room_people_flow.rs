use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        Clock, PortFuture, PrivateRoomSnapshot, PrivateRoomStore, PublicLobbyDirectoryEntry,
        PublicLobbyObservationRoom, RoomAgentKind, RoomAgentOwnerRecord, RoomDirectory,
        RoomDirectoryQuery, RoomPeopleDirectory, RoomPersonName,
    },
    room_people::{
        ListRoomPeople, LookupRoomAgentOwners, MAX_AGENT_OWNER_CANDIDATES, RoomPeopleDependencies,
        RoomPeopleFailure, RoomPeopleService, RoomPeopleUseCases,
    },
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId, RoomInstanceId},
    private_rooms::{PrivateRoom, PrivateRoomCapability, PrivateRoomPermissions},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState, RoomSlug,
    },
    time::UtcMillis,
    version::AggregateVersion,
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

#[derive(Default)]
struct 记录名录 {
    calls: Mutex<Vec<String>>,
}

impl RoomPeopleDirectory for 记录名录 {
    fn member_names(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomPersonName>>> {
        self.calls.lock().unwrap().push("names".to_owned());
        Box::pin(async { Ok(vec![person(1, "房主"), person(2, "ming@example.com")]) })
    }

    fn agent_owners<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        agent_ids: &'a [AgentId],
    ) -> PortFuture<'a, RepositoryResult<Vec<RoomAgentOwnerRecord>>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("owners {}", agent_ids.len()));
        Box::pin(async {
            Ok(vec![RoomAgentOwnerRecord {
                agent_id: AgentId::from_uuid(Uuid::from_u128(30)),
                kind: RoomAgentKind::Local,
                owner: Some(person(2, "ming@example.com")),
                admitted_by: None,
            }])
        })
    }
}

struct 固定私人房间(Option<PrivateRoomSnapshot>);

impl PrivateRoomStore for 固定私人房间 {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("只读") })
    }

    fn find_by_catalog(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        let value = self.0.clone();
        Box::pin(async move { Ok(value) })
    }

    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("只读一个房间") })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("按目录找房间") })
    }

    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("只读") })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("只读") })
    }
}

struct 固定目录(Option<RoomCatalog>);

impl RoomDirectory for 固定目录 {
    fn list_public<'a>(
        &'a self,
        _query: &'a RoomDirectoryQuery,
    ) -> PortFuture<'a, RepositoryResult<Vec<PublicLobbyDirectoryEntry>>> {
        Box::pin(async { unreachable!("不列大厅") })
    }

    fn find_catalog(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomCatalog>>> {
        let value = self.0.clone();
        Box::pin(async move { Ok(value) })
    }

    fn find_public_observation_room(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PublicLobbyObservationRoom>>> {
        Box::pin(async { unreachable!("不围观") })
    }
}

struct 固定时钟;

impl Clock for 固定时钟 {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

fn build(
    private: Option<PrivateRoomSnapshot>,
    catalog: Option<RoomCatalog>,
) -> (RoomPeopleService, Arc<记录名录>) {
    let people = Arc::new(记录名录::default());
    let service = RoomPeopleService::new(RoomPeopleDependencies {
        people: people.clone(),
        private_rooms: Arc::new(固定私人房间(private)),
        directory: Arc::new(固定目录(catalog)),
        clock: Arc::new(固定时钟),
    });
    (service, people)
}

#[tokio::test]
async fn 私人房间的成员看得到名字_带_at_的只留前面那段() {
    let snapshot = private_room(Some(member_principal()), true);
    let (service, people) = build(Some(snapshot.clone()), Some(snapshot.catalog().clone()));

    let names = service
        .people(ListRoomPeople {
            actor: actor(member_principal()),
            catalog_id: catalog_id(),
        })
        .await
        .expect("成员能看");

    assert_eq!(names, vec![person(1, "房主"), person(2, "ming")]);
    assert_eq!(*people.calls.lock().unwrap(), vec!["names".to_owned()]);
}

#[tokio::test]
async fn 只受邀和不在房间里的看不到名字也看不到主人() {
    for joined in [false, true] {
        let snapshot = private_room(Some(member_principal()), joined);
        let outsider = if joined {
            PrincipalId::from_uuid(Uuid::from_u128(99))
        } else {
            member_principal()
        };
        let (service, people) = build(Some(snapshot.clone()), Some(snapshot.catalog().clone()));

        let names = service
            .people(ListRoomPeople {
                actor: actor(outsider),
                catalog_id: catalog_id(),
            })
            .await
            .expect_err("看不到");
        let owners = service
            .agent_owners(owners_request(outsider, 1))
            .await
            .expect_err("看不到");

        assert_eq!(names, RoomPeopleFailure::NotFound);
        assert_eq!(owners, RoomPeopleFailure::NotFound);
        assert!(people.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn 私人房间的成员看得到_agent_的主人() {
    let snapshot = private_room(Some(member_principal()), true);
    let (service, people) = build(Some(snapshot.clone()), Some(snapshot.catalog().clone()));

    let owners = service
        .agent_owners(owners_request(member_principal(), 2))
        .await
        .expect("成员能看");

    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0].owner, Some(person(2, "ming")));
    assert_eq!(*people.calls.lock().unwrap(), vec!["owners 2".to_owned()]);
}

#[tokio::test]
async fn 公共大厅登录了就能问_停用的大厅和别的房间当作没有() {
    let stranger = PrincipalId::from_uuid(Uuid::from_u128(99));
    let (service, people) = build(None, Some(lobby(RoomCatalogStatus::Active)));
    service
        .agent_owners(owners_request(stranger, 1))
        .await
        .expect("登录了就能问");
    assert_eq!(*people.calls.lock().unwrap(), vec!["owners 1".to_owned()]);

    for catalog in [
        Some(lobby(RoomCatalogStatus::Frozen)),
        Some(lobby(RoomCatalogStatus::Archived)),
        None,
    ] {
        let (service, people) = build(None, catalog);
        let failure = service
            .agent_owners(owners_request(stranger, 1))
            .await
            .expect_err("当作没有");
        assert_eq!(failure, RoomPeopleFailure::NotFound);
        assert!(people.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn 公共大厅不回答成员名字() {
    let (service, people) = build(None, Some(lobby(RoomCatalogStatus::Active)));

    let failure = service
        .people(ListRoomPeople {
            actor: actor(member_principal()),
            catalog_id: catalog_id(),
        })
        .await
        .expect_err("公共大厅没有成员名单");

    assert_eq!(failure, RoomPeopleFailure::NotFound);
    assert!(people.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 一个都没问时不查_问太多时拒绝_登录过期拒绝() {
    let (service, people) = build(None, Some(lobby(RoomCatalogStatus::Active)));
    let empty = service
        .agent_owners(owners_request(member_principal(), 0))
        .await
        .expect("没问就是没有");
    assert!(empty.is_empty());

    let too_many = service
        .agent_owners(owners_request(
            member_principal(),
            MAX_AGENT_OWNER_CANDIDATES + 1,
        ))
        .await
        .expect_err("太多");
    assert_eq!(too_many, RoomPeopleFailure::TooMany);

    let mut expired = actor(member_principal());
    expired.expires_at = time(NOW);
    let failure = service
        .agent_owners(LookupRoomAgentOwners {
            actor: expired,
            catalog_id: catalog_id(),
            agent_ids: vec![AgentId::from_uuid(Uuid::from_u128(30))],
        })
        .await
        .expect_err("过期了");
    assert_eq!(failure, RoomPeopleFailure::Forbidden);
    assert!(people.calls.lock().unwrap().is_empty());
}

fn owners_request(principal_id: PrincipalId, count: usize) -> LookupRoomAgentOwners {
    LookupRoomAgentOwners {
        actor: actor(principal_id),
        catalog_id: catalog_id(),
        agent_ids: (0..count)
            .map(|index| AgentId::from_uuid(Uuid::from_u128(1_000 + index as u128)))
            .collect(),
    }
}

/// 房主是 1 号；`member` 受邀，`joined` 时接受了邀请。
fn private_room(member: Option<PrincipalId>, joined: bool) -> PrivateRoomSnapshot {
    let owner = PrincipalId::from_uuid(Uuid::from_u128(1));
    let mut room = PrivateRoom::create(catalog_id(), owner);
    if let Some(principal_id) = member {
        room.invite(owner, principal_id, view_and_speak())
            .expect("邀请成功");
        if joined {
            room.accept_invitation(principal_id).expect("接受邀请成功");
        }
    }
    let catalog = catalog(
        RoomCatalogKind::PrivateRoom,
        RoomCatalogStatus::Active,
        Some(owner),
    );
    let instance = RoomInstance::restore(
        RoomInstanceId::from_uuid(Uuid::from_u128(11)),
        RoomInstanceFields {
            catalog_id: catalog_id(),
            matrix_room_id: MatrixRoomReference::new("!project:matrix.test".to_owned())
                .expect("房间标识有效"),
            region: None,
            capacity: RoomCapacity::standard(),
            projected_member_count: 0,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .expect("私人实例有效");
    PrivateRoomSnapshot::new(catalog, instance, room).expect("快照一致")
}

fn lobby(status: RoomCatalogStatus) -> RoomCatalog {
    catalog(RoomCatalogKind::PublicLobby, status, None)
}

fn catalog(
    kind: RoomCatalogKind,
    status: RoomCatalogStatus,
    owner: Option<PrincipalId>,
) -> RoomCatalog {
    RoomCatalog::new(
        catalog_id(),
        RoomCatalogFields {
            kind,
            slug: owner
                .is_none()
                .then(|| RoomSlug::new("lobby".to_owned()).expect("短名有效")),
            name: "项目室".to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: owner,
            visibility: if owner.is_some() {
                RoomCatalogVisibility::Private
            } else {
                RoomCatalogVisibility::Public
            },
            retention_days: Some(30),
            status,
        },
    )
    .expect("目录有效")
}

fn view_and_speak() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
    ])
    .expect("权限有效")
}

fn person(value: u128, name: &str) -> RoomPersonName {
    RoomPersonName {
        principal_id: PrincipalId::from_uuid(Uuid::from_u128(value)),
        display_name: name.to_owned(),
    }
}

fn member_principal() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(2))
}

fn actor(principal_id: PrincipalId) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id,
        matrix_user_id: "@user:matrix.test".to_owned(),
        display_name: "User".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: time(NOW + 60_000),
        recently_authenticated: false,
    }
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(10))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("时间有效")
}
