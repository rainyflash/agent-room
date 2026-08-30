use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        PortFuture, PublicLobbyDirectoryEntry, PublicLobbyObservationRoom, RoomDirectory,
        RoomDirectoryQuery,
    },
    public_lobby_entry::{
        EnterPublicLobby, PublicLobbyEntryDependencies, PublicLobbyEntryFailureKind,
        PublicLobbyEntryService, PublicLobbyEntryUseCases,
    },
    rooms::{
        LobbyProvisioningOperation, LobbyProvisioningOutcome, LobbyProvisioningRequest,
        LobbyProvisioningResult, ProvisionedLobby,
    },
};
use agent_room_domain::{
    ids::{RoomCatalogId, RoomInstanceId},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState, RoomLanguage, RoomSlug,
    },
    time::UtcMillis,
};
use uuid::Uuid;

const CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const INSTANCE_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";

#[tokio::test]
async fn 已有云端房间时直接返回且不重复供给() {
    let expected = observation_room();
    let directory = Arc::new(FakeDirectory::new(Some(expected.clone()), Some(catalog())));
    let provisioning = Arc::new(FakeProvisioning::ready());
    let result = service(directory.clone(), provisioning.clone())
        .enter(EnterPublicLobby {
            catalog_id: catalog_id(),
        })
        .await;

    assert_eq!(result, Ok(expected));
    assert_eq!(directory.catalog_reads.load(Ordering::SeqCst), 0);
    assert_eq!(provisioning.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn 没有_agent_在线时由云端幂等供给真实房间() {
    let directory = Arc::new(FakeDirectory::new(None, Some(catalog())));
    let provisioning = Arc::new(FakeProvisioning::ready());
    let result = service(directory.clone(), provisioning.clone())
        .enter(EnterPublicLobby {
            catalog_id: catalog_id(),
        })
        .await
        .expect("云端应完成公开大厅供给");

    assert_eq!(result, observation_room());
    assert_eq!(directory.catalog_reads.load(Ordering::SeqCst), 1);
    assert_eq!(provisioning.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn 不存在的目录不触发_matrix_供给() {
    let directory = Arc::new(FakeDirectory::new(None, None));
    let provisioning = Arc::new(FakeProvisioning::ready());
    let failure = service(directory, provisioning.clone())
        .enter(EnterPublicLobby {
            catalog_id: catalog_id(),
        })
        .await
        .expect_err("不存在的目录必须失败关闭");

    assert_eq!(failure.kind(), PublicLobbyEntryFailureKind::NotFound);
    assert_eq!(provisioning.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn 并发供给返回明确重试时间() {
    let retry_at = UtcMillis::new(42_000).expect("重试时间有效");
    let directory = Arc::new(FakeDirectory::new(None, Some(catalog())));
    let provisioning = Arc::new(FakeProvisioning {
        calls: AtomicUsize::new(0),
        outcome: Ok(LobbyProvisioningOutcome::Busy { retry_at }),
    });
    let failure = service(directory, provisioning)
        .enter(EnterPublicLobby {
            catalog_id: catalog_id(),
        })
        .await
        .expect_err("并发供给应要求重试");

    assert_eq!(
        failure.kind(),
        PublicLobbyEntryFailureKind::ProvisioningBusy
    );
    assert_eq!(failure.retry_at(), Some(retry_at));
}

fn service(
    directory: Arc<FakeDirectory>,
    provisioning: Arc<FakeProvisioning>,
) -> PublicLobbyEntryService {
    PublicLobbyEntryService::new(PublicLobbyEntryDependencies {
        directory,
        provisioning,
    })
}

struct FakeDirectory {
    catalog: Option<RoomCatalog>,
    catalog_reads: AtomicUsize,
    observation: Option<PublicLobbyObservationRoom>,
}

impl FakeDirectory {
    fn new(observation: Option<PublicLobbyObservationRoom>, catalog: Option<RoomCatalog>) -> Self {
        Self {
            catalog,
            catalog_reads: AtomicUsize::new(0),
            observation,
        }
    }
}

impl RoomDirectory for FakeDirectory {
    fn list_public<'a>(
        &'a self,
        _query: &'a RoomDirectoryQuery,
    ) -> PortFuture<'a, RepositoryResult<Vec<PublicLobbyDirectoryEntry>>> {
        Box::pin(async { unreachable!("公开大厅入场不读取目录列表") })
    }

    fn find_catalog(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomCatalog>>> {
        self.catalog_reads.fetch_add(1, Ordering::SeqCst);
        let catalog = self.catalog.clone();
        Box::pin(async move { Ok(catalog) })
    }

    fn find_public_observation_room(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PublicLobbyObservationRoom>>> {
        let observation = self.observation.clone();
        Box::pin(async move { Ok(observation) })
    }
}

struct FakeProvisioning {
    calls: AtomicUsize,
    outcome: LobbyProvisioningResult<LobbyProvisioningOutcome>,
}

impl FakeProvisioning {
    fn ready() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            outcome: Ok(LobbyProvisioningOutcome::Ready(Box::new(
                ProvisionedLobby {
                    catalog: catalog(),
                    room: room(),
                },
            ))),
        }
    }
}

impl LobbyProvisioningOperation for FakeProvisioning {
    fn provision(
        &self,
        _request: LobbyProvisioningRequest,
    ) -> PortFuture<'_, LobbyProvisioningResult<LobbyProvisioningOutcome>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self.outcome.clone();
        Box::pin(async move { outcome })
    }
}

fn observation_room() -> PublicLobbyObservationRoom {
    PublicLobbyObservationRoom {
        catalog_id: catalog_id(),
        room_instance_id: room_instance_id(),
        matrix_room_id: matrix_room_reference(),
    }
}

fn catalog() -> RoomCatalog {
    RoomCatalog::new(
        catalog_id(),
        RoomCatalogFields {
            kind: RoomCatalogKind::PublicLobby,
            slug: Some(RoomSlug::new("general").expect("短名有效")),
            name: "General".to_owned(),
            description: "Public Agent Room lobby".to_owned(),
            language: Some(RoomLanguage::new("en").expect("语言有效")),
            matrix_space_id: Some(
                MatrixRoomReference::new("!space:matrix.agent-room.test".to_owned())
                    .expect("Space 标识有效"),
            ),
            owner_principal_id: None,
            visibility: RoomCatalogVisibility::Public,
            retention_days: None,
            status: RoomCatalogStatus::Active,
        },
    )
    .expect("公开目录有效")
}

fn room() -> RoomInstance {
    RoomInstance::restore(
        room_instance_id(),
        RoomInstanceFields {
            catalog_id: catalog_id(),
            matrix_room_id: matrix_room_reference(),
            region: None,
            capacity: RoomCapacity::standard(),
            projected_member_count: 0,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .expect("房间有效")
}

fn matrix_room_reference() -> MatrixRoomReference {
    MatrixRoomReference::new("!public-lobby:matrix.agent-room.test".to_owned())
        .expect("房间标识有效")
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::parse_str(CATALOG_ID).expect("目录 UUID 有效"))
}

fn room_instance_id() -> RoomInstanceId {
    RoomInstanceId::from_uuid(Uuid::parse_str(INSTANCE_ID).expect("实例 UUID 有效"))
}
