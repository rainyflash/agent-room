use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use agent_room_application::{
    agent_cards::{
        AgentCardChange, AgentCardDependencies, AgentCardManagementFailureKind, AgentCardService,
        AgentCardUseCases, RefreshAgentCard,
    },
    devices::AuthenticatedDevice,
    persistence::RepositoryResult,
    ports::{
        AgentCardFetchResult, AgentCardSnapshotRepository, AgentCardSource,
        AgentMembershipRepository, AgentRegistration, AgentRepository, Clock, FetchedAgentCard,
        IdentifierFactory, PortFuture, PrincipalAccount, RegisteredAgent,
    },
};
use agent_room_domain::{
    agent_cards::{
        AgentCardCapabilities, AgentCardDigest, AgentCardEndpoint, AgentCardProtocolVersion,
        AgentCardSnapshot, AgentCardSnapshotFields, AgentCardSourceUrl, AgentCardTransport,
        AgentCardVerificationState, AgentEndpointVerificationState, NormalizedAgentCard,
        NormalizedAgentCardFields,
    },
    agents::{Agent, AgentMemberships, AgentRole},
    identity::Principal,
    ids::{
        AdapterBindingId, AgentCardSnapshotId, AgentId, AgentInstanceId, AutomationGrantId,
        ContentId, DeviceAccessTokenId, DeviceId, DeviceRefreshTokenId, DeviceTokenFamilyId,
        HandoffId, LoginAttemptId, OutboxEventId, PrincipalId, RoomCatalogId, RoomInstanceId,
        RoomReservationId, WebSessionId,
    },
    time::{DurationMillis, UtcMillis},
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

struct StaticClock;

impl Clock for StaticClock {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

struct TestIdentifiers;

impl IdentifierFactory for TestIdentifiers {
    fn principal_id(&self) -> PrincipalId {
        PrincipalId::from_uuid(Uuid::now_v7())
    }

    fn login_attempt_id(&self) -> LoginAttemptId {
        LoginAttemptId::from_uuid(Uuid::now_v7())
    }

    fn web_session_id(&self) -> WebSessionId {
        WebSessionId::from_uuid(Uuid::now_v7())
    }

    fn device_id(&self) -> DeviceId {
        DeviceId::from_uuid(Uuid::now_v7())
    }

    fn device_token_family_id(&self) -> DeviceTokenFamilyId {
        DeviceTokenFamilyId::from_uuid(Uuid::now_v7())
    }

    fn device_access_token_id(&self) -> DeviceAccessTokenId {
        DeviceAccessTokenId::from_uuid(Uuid::now_v7())
    }

    fn device_refresh_token_id(&self) -> DeviceRefreshTokenId {
        DeviceRefreshTokenId::from_uuid(Uuid::now_v7())
    }

    fn agent_id(&self) -> AgentId {
        AgentId::from_uuid(Uuid::now_v7())
    }

    fn agent_card_snapshot_id(&self) -> AgentCardSnapshotId {
        AgentCardSnapshotId::from_uuid(Uuid::now_v7())
    }

    fn adapter_binding_id(&self) -> AdapterBindingId {
        AdapterBindingId::from_uuid(Uuid::now_v7())
    }

    fn agent_instance_id(&self) -> AgentInstanceId {
        AgentInstanceId::from_uuid(Uuid::now_v7())
    }

    fn room_catalog_id(&self) -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::now_v7())
    }

    fn room_instance_id(&self) -> RoomInstanceId {
        RoomInstanceId::from_uuid(Uuid::now_v7())
    }

    fn room_reservation_id(&self) -> RoomReservationId {
        RoomReservationId::from_uuid(Uuid::now_v7())
    }

    fn content_id(&self) -> ContentId {
        ContentId::from_uuid(Uuid::now_v7())
    }

    fn handoff_id(&self) -> HandoffId {
        HandoffId::from_uuid(Uuid::now_v7())
    }

    fn automation_grant_id(&self) -> AutomationGrantId {
        AutomationGrantId::from_uuid(Uuid::now_v7())
    }

    fn outbox_event_id(&self) -> OutboxEventId {
        OutboxEventId::from_uuid(Uuid::now_v7())
    }
}

struct FakeAgentRepository {
    agent: Agent,
}

impl AgentRepository for FakeAgentRepository {
    fn find(&self, id: AgentId) -> PortFuture<'_, RepositoryResult<Option<Agent>>> {
        let value = (self.agent.id() == id).then(|| self.agent.clone());
        Box::pin(async move { Ok(value) })
    }

    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RegisteredAgent>>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn find_registration(
        &self,
        _id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<RegisteredAgent>>> {
        Box::pin(async { Ok(None) })
    }

    fn create<'a>(
        &'a self,
        registration: &'a AgentRegistration,
    ) -> PortFuture<'a, RepositoryResult<Agent>> {
        Box::pin(async move { Ok(registration.agent.clone()) })
    }

    fn save<'a>(&'a self, agent: &'a Agent) -> PortFuture<'a, RepositoryResult<Agent>> {
        Box::pin(async move { Ok(agent.clone()) })
    }
}

struct FakeMemberships {
    memberships: AgentMemberships,
}

impl AgentMembershipRepository for FakeMemberships {
    fn find_memberships(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentMemberships>>> {
        let value = (self.memberships.agent_id() == agent_id).then(|| self.memberships.clone());
        Box::pin(async move { Ok(value) })
    }
}

struct FakeSource {
    fetched: FetchedAgentCard,
    calls: AtomicUsize,
}

impl AgentCardSource for FakeSource {
    fn fetch<'a>(
        &'a self,
        _source_url: &'a AgentCardSourceUrl,
    ) -> PortFuture<'a, AgentCardFetchResult<FetchedAgentCard>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let fetched = self.fetched.clone();
        Box::pin(async move { Ok(fetched) })
    }
}

struct FakeSnapshots {
    latest: Option<AgentCardSnapshot>,
    saved: Mutex<Vec<AgentCardSnapshot>>,
}

impl AgentCardSnapshotRepository for FakeSnapshots {
    fn find_latest(
        &self,
        _agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentCardSnapshot>>> {
        let latest = self.latest.clone();
        Box::pin(async move { Ok(latest) })
    }

    fn save<'a>(
        &'a self,
        snapshot: &'a AgentCardSnapshot,
    ) -> PortFuture<'a, RepositoryResult<AgentCardSnapshot>> {
        Box::pin(async move {
            self.saved
                .lock()
                .expect("测试快照锁不得中毒")
                .push(snapshot.clone());
            Ok(snapshot.clone())
        })
    }
}

#[tokio::test]
async fn owner_刷新后保存有界快照并计算过期时间() {
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let owner_id = PrincipalId::from_uuid(Uuid::now_v7());
    let source = Arc::new(FakeSource {
        fetched: fetched_card("研究助手", true, [7; 32]),
        calls: AtomicUsize::new(0),
    });
    let snapshots = Arc::new(FakeSnapshots {
        latest: None,
        saved: Mutex::new(Vec::new()),
    });
    let service = service(
        agent_id,
        AgentMemberships::with_initial_owner(agent_id, owner_id, time(NOW)),
        source.clone(),
        snapshots.clone(),
    );

    let result = service
        .refresh(refresh_request(agent_id, owner_id))
        .await
        .expect("Owner 可刷新 Card");

    assert_eq!(result.change, AgentCardChange::Initial);
    assert_eq!(result.snapshot.expires_at(), time(NOW + 60_000));
    assert_eq!(result.snapshot.card().name(), "研究助手");
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert_eq!(snapshots.saved.lock().expect("测试快照锁不得中毒").len(), 1);
}

#[tokio::test]
async fn viewer_在网络请求前被拒绝() {
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let owner_id = PrincipalId::from_uuid(Uuid::now_v7());
    let viewer_id = PrincipalId::from_uuid(Uuid::now_v7());
    let mut memberships = AgentMemberships::with_initial_owner(agent_id, owner_id, time(NOW));
    memberships
        .grant_role(owner_id, viewer_id, AgentRole::Viewer, time(NOW))
        .expect("测试 Viewer 可添加");
    let source = Arc::new(FakeSource {
        fetched: fetched_card("研究助手", true, [7; 32]),
        calls: AtomicUsize::new(0),
    });
    let service = service(
        agent_id,
        memberships,
        source.clone(),
        Arc::new(FakeSnapshots {
            latest: None,
            saved: Mutex::new(Vec::new()),
        }),
    );

    let failure = service
        .refresh(refresh_request(agent_id, viewer_id))
        .await
        .expect_err("Viewer 不得刷新 Card");

    assert_eq!(failure.kind(), AgentCardManagementFailureKind::Forbidden);
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn 能力变化与纯资料变化被明确区分() {
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let owner_id = PrincipalId::from_uuid(Uuid::now_v7());
    let memberships = AgentMemberships::with_initial_owner(agent_id, owner_id, time(NOW));
    let latest = snapshot(agent_id, card("旧名称", false), [1; 32]);
    let capability_service = service(
        agent_id,
        memberships.clone(),
        Arc::new(FakeSource {
            fetched: fetched_card("新名称", true, [2; 32]),
            calls: AtomicUsize::new(0),
        }),
        Arc::new(FakeSnapshots {
            latest: Some(latest.clone()),
            saved: Mutex::new(Vec::new()),
        }),
    );
    let capability_change = capability_service
        .refresh(refresh_request(agent_id, owner_id))
        .await
        .expect("能力变化可刷新");
    assert_eq!(
        capability_change.change,
        AgentCardChange::CapabilitySurfaceChanged
    );

    let profile_service = service(
        agent_id,
        memberships,
        Arc::new(FakeSource {
            fetched: fetched_card("新名称", false, [3; 32]),
            calls: AtomicUsize::new(0),
        }),
        Arc::new(FakeSnapshots {
            latest: Some(latest),
            saved: Mutex::new(Vec::new()),
        }),
    );
    let profile_change = profile_service
        .refresh(refresh_request(agent_id, owner_id))
        .await
        .expect("资料变化可刷新");
    assert_eq!(profile_change.change, AgentCardChange::ProfileChanged);
}

fn service(
    agent_id: AgentId,
    memberships: AgentMemberships,
    source: Arc<dyn AgentCardSource>,
    snapshots: Arc<dyn AgentCardSnapshotRepository>,
) -> AgentCardService {
    AgentCardService::new(AgentCardDependencies {
        agents: Arc::new(FakeAgentRepository {
            agent: Agent::register(agent_id),
        }),
        memberships: Arc::new(FakeMemberships { memberships }),
        source,
        snapshots,
        identifiers: Arc::new(TestIdentifiers),
        clock: Arc::new(StaticClock),
    })
}

fn refresh_request(agent_id: AgentId, principal_id: PrincipalId) -> RefreshAgentCard {
    RefreshAgentCard {
        actor: AuthenticatedDevice {
            account: PrincipalAccount {
                principal: Principal::new(principal_id),
                matrix_user_id: "@owner:matrix.agent-room.test".to_owned(),
                display_name: "Owner".to_owned(),
                avatar_content_id: None,
                locale: "zh-CN".to_owned(),
            },
            device_id: DeviceId::from_uuid(Uuid::now_v7()),
            access_token_expires_at: time(NOW + 300_000),
        },
        agent_id,
        source_url: source_url(),
    }
}

fn fetched_card(name: &str, streaming: bool, digest: [u8; 32]) -> FetchedAgentCard {
    FetchedAgentCard {
        digest: AgentCardDigest::from_array(digest),
        card: card(name, streaming),
        verification: AgentCardVerificationState::Unverified,
        cache_lifetime: DurationMillis::new(60_000).expect("测试缓存时限有效"),
    }
}

fn snapshot(agent_id: AgentId, card: NormalizedAgentCard, digest: [u8; 32]) -> AgentCardSnapshot {
    AgentCardSnapshot::new(AgentCardSnapshotFields {
        id: AgentCardSnapshotId::from_uuid(Uuid::now_v7()),
        agent_id,
        source_url: source_url(),
        digest: AgentCardDigest::from_array(digest),
        card,
        verification: AgentCardVerificationState::Unverified,
        fetched_at: time(NOW - 60_000),
        expires_at: time(NOW + 60_000),
    })
    .expect("测试快照有效")
}

fn card(name: &str, streaming: bool) -> NormalizedAgentCard {
    let endpoint = AgentCardEndpoint::new(
        "https://agent.example/a2a".to_owned(),
        AgentCardTransport::HttpJson,
        AgentCardProtocolVersion::V1_0,
        None,
        AgentEndpointVerificationState::Declared,
    )
    .expect("测试端点有效");
    NormalizedAgentCard::new(NormalizedAgentCardFields {
        name: name.to_owned(),
        description: "公开资料".to_owned(),
        provider: None,
        version: "1.0.0".to_owned(),
        endpoints: vec![endpoint],
        capabilities: AgentCardCapabilities::new(
            streaming,
            false,
            false,
            Vec::new(),
            &BTreeSet::new(),
        )
        .expect("测试能力有效"),
        security_schemes: Vec::new(),
        default_input_modes: vec!["text/plain".to_owned()],
        default_output_modes: vec!["text/plain".to_owned()],
        skills: Vec::new(),
    })
    .expect("测试 Card 有效")
}

fn source_url() -> AgentCardSourceUrl {
    AgentCardSourceUrl::new("https://agent.example/.well-known/agent-card.json".to_owned())
        .expect("测试来源有效")
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
