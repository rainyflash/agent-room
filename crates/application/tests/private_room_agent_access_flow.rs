//! 私人房间 Agent 口令：房主生成与停用、Agent 凭口令加入、猜错限流、移出与再加入。

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    persistence::RepositoryResult,
    ports::{
        AgentMembershipRepository, Clock, JoinCodeAttemptPolicy, MatrixResult, MatrixRoomId,
        MatrixUserId, PortFuture, PrincipalAccount, PrivateMatrixMembership,
        PrivateMatrixSpeakingAssignment, PrivateRoomAgentAccessStore, PrivateRoomAgentMemberRecord,
        PrivateRoomJoinCodeRecord, PrivateRoomMatrixGateway, PrivateRoomSnapshot, PrivateRoomStore,
        SecretDigest, SecretFactory, SecretGenerationFailure, SecretValue,
    },
    private_rooms::{
        AgentAccessFailureKind, InspectAgentAccess, JoinCodeCaller, ManageJoinCode,
        PrivateRoomAgentAccessDependencies, PrivateRoomAgentAccessService,
        PrivateRoomAgentAccessUseCases, RedeemJoinCode, RemoveAgentMember, ResolveJoinCode,
    },
};
use agent_room_domain::{
    agents::AgentMemberships,
    identity::Principal,
    ids::{AgentId, DeviceId, PrincipalId, RoomCatalogId, RoomInstanceId},
    join_codes::{PrivateRoomAgentMemberStatus, PrivateRoomJoinCode},
    private_rooms::{
        PrivateRoom, PrivateRoomCapability, PrivateRoomLifecycleStatus, PrivateRoomPermissions,
    },
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
    },
    time::{DurationMillis, UtcMillis},
    version::AggregateVersion,
};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

const NOW: i64 = 1_790_000_000_000;
const HOUR: i64 = 3_600_000;

#[tokio::test]
async fn 房主生成口令_再生成就替换旧的_查看时不含口令本身() {
    let fixture = Fixture::new();
    let first = fixture
        .service
        .generate_code(manage(owner()))
        .await
        .expect("房主可以生成口令");
    assert_eq!(
        PrivateRoomJoinCode::parse(&first.code.display()).unwrap(),
        first.code
    );
    assert_eq!(
        first.record.permissions,
        PrivateRoomPermissions::AGENT_MEMBER
    );
    fixture.clock.advance(1_000);
    let second = fixture
        .service
        .generate_code(manage(owner()))
        .await
        .expect("可以更换口令");
    assert_ne!(first.code, second.code);
    assert!(fixture.store.has_digest(&second.code));
    assert!(!fixture.store.has_digest(&first.code), "旧口令随之失效");

    let view = fixture
        .service
        .inspect(InspectAgentAccess {
            actor: principal_actor(owner()),
            catalog_id: catalog_id(),
        })
        .await
        .expect("房主可以查看");
    assert_eq!(view.join_code, Some(second.record));
    assert!(view.agents.is_empty());
}

#[tokio::test]
async fn 只有能管理的成员可以管理口令_归档房间不能() {
    let fixture = Fixture::new();
    let member = fixture
        .service
        .generate_code(manage(speaker()))
        .await
        .expect_err("普通成员不能管理");
    assert_eq!(member.kind(), AgentAccessFailureKind::Forbidden);
    let stranger = fixture
        .service
        .generate_code(manage(PrincipalId::from_uuid(Uuid::from_u128(99))))
        .await
        .expect_err("看不见这个房间的人不知道它存在");
    assert_eq!(stranger.kind(), AgentAccessFailureKind::NotFound);

    let archived = Fixture::archived();
    let failure = archived
        .service
        .generate_code(manage(owner()))
        .await
        .expect_err("归档房间不再生成口令");
    assert_eq!(failure.kind(), AgentAccessFailureKind::Conflict);
}

#[tokio::test]
async fn 凭口令加入后记为_agent_成员并返回房间() {
    let fixture = Fixture::new();
    let generated = fixture
        .service
        .generate_code(manage(owner()))
        .await
        .expect("生成口令");
    // 大小写和分隔都不影响。
    let room = fixture
        .service
        .redeem(redeem_request(&generated.code.display().to_lowercase()))
        .await
        .expect("凭口令加入");
    assert_eq!(room.catalog_id, catalog_id());
    assert_eq!(room.matrix_room_id, matrix_room());
    assert_eq!(room.name, "项目室");
    assert_eq!(
        fixture.store.status(agent_id()),
        Some(PrivateRoomAgentMemberStatus::Joined)
    );
    // 重复兑换幂等。
    fixture
        .service
        .redeem(redeem_request(generated.code.normalized()))
        .await
        .expect("重复兑换");
    let view = fixture
        .service
        .inspect(InspectAgentAccess {
            actor: principal_actor(owner()),
            catalog_id: catalog_id(),
        })
        .await
        .expect("查看");
    assert_eq!(view.agents.len(), 1);
    assert_eq!(view.agents[0].display_name, "Scout");
}

#[tokio::test]
async fn 只能带自己的_agent_进来_且不计猜测次数() {
    let fixture = Fixture::new();
    let generated = fixture
        .service
        .generate_code(manage(owner()))
        .await
        .expect("生成口令");
    // 设备所属账号不是这个 Agent 的主人或操作者。
    let mut request = redeem_request(generated.code.normalized());
    request.actor.account.principal = Principal::new(speaker());
    let failure = fixture
        .service
        .redeem(request)
        .await
        .expect_err("不能带别人的 Agent 进来");
    assert_eq!(failure.kind(), AgentAccessFailureKind::Forbidden);
    let mut request = redeem_request(generated.code.normalized());
    request.agent_id = AgentId::from_uuid(Uuid::from_u128(405));
    assert_eq!(
        fixture.service.redeem(request).await.unwrap_err().kind(),
        AgentAccessFailureKind::NotFound
    );
    assert_eq!(fixture.store.failures(), 0);
    assert_eq!(fixture.store.status(agent_id()), None);
}

#[tokio::test]
async fn 查看口令对应的房间不会让任何_agent_加入_猜错同样计次() {
    let fixture = Fixture::new();
    let generated = fixture
        .service
        .generate_code(manage(owner()))
        .await
        .expect("生成口令");
    let room = fixture
        .service
        .resolve(ResolveJoinCode {
            caller: JoinCodeCaller::Device(redeem_request("").actor),
            code: generated.code.display(),
        })
        .await
        .expect("查看房间");
    assert_eq!(room.catalog_id, catalog_id());
    assert_eq!(room.matrix_room_id, matrix_room());
    assert_eq!(room.name, "项目室");
    assert_eq!(fixture.store.status(agent_id()), None, "只看不加入");
    let wrong = fixture
        .service
        .resolve(ResolveJoinCode {
            caller: JoinCodeCaller::Device(redeem_request("").actor),
            code: "0000-0000-0000".to_owned(),
        })
        .await
        .expect_err("猜错");
    assert_eq!(wrong.kind(), AgentAccessFailureKind::NotFound);
    assert_eq!(fixture.store.failures(), 1);
}

#[tokio::test]
async fn 格式不对不算猜测_猜错到上限后限流_窗口过后恢复() {
    let fixture = Fixture::new();
    let generated = fixture
        .service
        .generate_code(manage(owner()))
        .await
        .expect("生成口令");
    let malformed = fixture
        .service
        .redeem(redeem_request("不是口令"))
        .await
        .expect_err("格式不对");
    assert_eq!(malformed.kind(), AgentAccessFailureKind::InvalidRequest);
    assert_eq!(fixture.store.failures(), 0);
    for _ in 0..3 {
        let wrong = fixture
            .service
            .redeem(redeem_request("0000-0000-0000"))
            .await
            .expect_err("猜错");
        assert_eq!(wrong.kind(), AgentAccessFailureKind::NotFound);
    }
    // 达到上限后连正确的口令也要等到窗口结束。
    let limited = fixture
        .service
        .redeem(redeem_request(generated.code.normalized()))
        .await
        .expect_err("限流");
    assert_eq!(limited.kind(), AgentAccessFailureKind::RateLimited);
    assert_eq!(limited.retry_at(), Some(time(NOW + HOUR)));
    fixture.clock.advance(HOUR);
    fixture
        .service
        .redeem(redeem_request(generated.code.normalized()))
        .await
        .expect("窗口过后可以再试");
}

#[tokio::test]
async fn 网络_agent_猜错按来源计数_与本机设备和别的来源分开() {
    let fixture = Fixture::new();
    let generated = fixture
        .service
        .generate_code(manage(owner()))
        .await
        .expect("生成口令");
    let resolve = |source: u8, code: String| {
        fixture.service.resolve(ResolveJoinCode {
            caller: JoinCodeCaller::NetworkSource([source; 32]),
            code,
        })
    };
    for _ in 0..3 {
        let wrong = resolve(7, "0000-0000-0000".to_owned())
            .await
            .expect_err("猜错");
        assert_eq!(wrong.kind(), AgentAccessFailureKind::NotFound);
    }
    assert_eq!(
        fixture.store.callers(),
        [format!("network-source:{}", "07".repeat(32))],
        "按来源计数，不落到任何设备上"
    );
    let limited = resolve(7, generated.code.display())
        .await
        .expect_err("这个来源猜错到了上限");
    assert_eq!(limited.kind(), AgentAccessFailureKind::RateLimited);

    // 别的来源、本机设备都不受影响。
    let room = resolve(8, generated.code.display())
        .await
        .expect("换个来源可以");
    assert_eq!(room.catalog_id, catalog_id());
    fixture
        .service
        .redeem(redeem_request(generated.code.normalized()))
        .await
        .expect("本机设备照常兑换");
}

#[tokio::test]
async fn 移出先收紧_matrix_再记账_旧口令进不来_新口令可以() {
    let fixture = Fixture::new();
    let old = fixture
        .service
        .generate_code(manage(owner()))
        .await
        .expect("生成口令");
    fixture
        .service
        .redeem(redeem_request(old.code.normalized()))
        .await
        .expect("凭口令加入");
    fixture.clock.advance(1_000);
    fixture
        .service
        .remove_agent(RemoveAgentMember {
            actor: principal_actor(owner()),
            catalog_id: catalog_id(),
            agent_id: agent_id(),
        })
        .await
        .expect("房主可以移出");
    assert_eq!(
        fixture.matrix.events(),
        ["speaking:false", "kick"],
        "先收回发言再踢出"
    );
    assert_eq!(
        fixture.store.status(agent_id()),
        Some(PrivateRoomAgentMemberStatus::Removed)
    );
    let refused = fixture
        .service
        .redeem(redeem_request(old.code.normalized()))
        .await
        .expect_err("移出前的口令不能再用");
    assert_eq!(refused.kind(), AgentAccessFailureKind::Forbidden);
    fixture.clock.advance(1_000);
    let newer = fixture
        .service
        .generate_code(manage(owner()))
        .await
        .expect("换新口令");
    fixture
        .service
        .redeem(redeem_request(newer.code.normalized()))
        .await
        .expect("移出之后生成的口令可以再进");
    assert_eq!(
        fixture.store.status(agent_id()),
        Some(PrivateRoomAgentMemberStatus::Joined)
    );
    // 移出不存在的 Agent 成员。
    let missing = fixture
        .service
        .remove_agent(RemoveAgentMember {
            actor: principal_actor(owner()),
            catalog_id: catalog_id(),
            agent_id: AgentId::from_uuid(Uuid::from_u128(77)),
        })
        .await
        .expect_err("不存在");
    assert_eq!(missing.kind(), AgentAccessFailureKind::NotFound);
}

#[tokio::test]
async fn 停用口令后兑换失败_归档房间的口令也不能用() {
    let fixture = Fixture::new();
    let generated = fixture
        .service
        .generate_code(manage(owner()))
        .await
        .expect("生成口令");
    fixture
        .service
        .disable_code(manage(owner()))
        .await
        .expect("停用口令");
    assert_eq!(
        fixture
            .service
            .redeem(redeem_request(generated.code.normalized()))
            .await
            .unwrap_err()
            .kind(),
        AgentAccessFailureKind::NotFound
    );

    let archived = Fixture::archived();
    let code = archived
        .store
        .install_code(PrivateRoomJoinCode::from_entropy([7; 8]));
    assert_eq!(
        archived
            .service
            .redeem(redeem_request(code.normalized()))
            .await
            .unwrap_err()
            .kind(),
        AgentAccessFailureKind::Conflict
    );
}

struct Fixture {
    service: PrivateRoomAgentAccessService,
    store: Arc<MemoryAccessStore>,
    matrix: Arc<RecordingMatrix>,
    clock: Arc<TestClock>,
}

impl Fixture {
    fn new() -> Self {
        Self::with_room(active_room())
    }

    fn archived() -> Self {
        let mut room = active_room();
        room.archive(owner()).expect("归档成功");
        Self::with_room(room)
    }

    fn with_room(room: PrivateRoom) -> Self {
        let store = Arc::new(MemoryAccessStore::default());
        let matrix = Arc::new(RecordingMatrix::default());
        let clock = Arc::new(TestClock(Mutex::new(NOW)));
        let service = PrivateRoomAgentAccessService::new(PrivateRoomAgentAccessDependencies {
            rooms: Arc::new(FixedRooms(snapshot(room))),
            access: store.clone(),
            memberships: Arc::new(FixedMemberships),
            matrix: matrix.clone(),
            secrets: Arc::new(CountingSecrets(Mutex::new(0))),
            clock: clock.clone(),
            attempts: JoinCodeAttemptPolicy {
                window: DurationMillis::new(u64::try_from(HOUR).unwrap()).unwrap(),
                max_failures: 3,
            },
        });
        Self {
            service,
            store,
            matrix,
            clock,
        }
    }
}

fn manage(actor: PrincipalId) -> ManageJoinCode {
    ManageJoinCode {
        actor: principal_actor(actor),
        catalog_id: catalog_id(),
    }
}

fn redeem_request(code: &str) -> RedeemJoinCode {
    RedeemJoinCode {
        actor: AuthenticatedDevice {
            account: PrincipalAccount {
                principal: Principal::new(outsider()),
                matrix_user_id: "@outsider:matrix.test".to_owned(),
                display_name: "Outsider".to_owned(),
                avatar_content_id: None,
                locale: "zh-CN".to_owned(),
            },
            device_id: device_id(),
            access_token_expires_at: UtcMillis::new(i64::MAX).unwrap(),
        },
        agent_id: agent_id(),
        code: code.to_owned(),
    }
}

fn active_room() -> PrivateRoom {
    let mut room = PrivateRoom::create(catalog_id(), owner());
    room.invite(
        owner(),
        speaker(),
        PrivateRoomPermissions::from_capabilities([
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
        ])
        .unwrap(),
    )
    .unwrap();
    room.accept_invitation(speaker()).unwrap();
    room
}

fn snapshot(room: PrivateRoom) -> PrivateRoomSnapshot {
    // 目录、实例和房间的生命周期必须一致。
    let archived = room.status() == PrivateRoomLifecycleStatus::Archived;
    let catalog = RoomCatalog::new(
        catalog_id(),
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: "项目室".to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(owner()),
            visibility: RoomCatalogVisibility::Private,
            retention_days: Some(30),
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
            matrix_room_id: matrix_room(),
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

fn principal_actor(principal_id: PrincipalId) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id,
        matrix_user_id: "@someone:matrix.test".to_owned(),
        display_name: "Someone".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: UtcMillis::new(i64::MAX).unwrap(),
        recently_authenticated: false,
    }
}

fn owner() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(1))
}

fn speaker() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(2))
}

/// 凭口令带 Agent 进来的人，本身不是房间成员。
fn outsider() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(3))
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(10))
}

fn matrix_room() -> MatrixRoomReference {
    MatrixRoomReference::new("!project:matrix.test".to_owned()).unwrap()
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(Uuid::from_u128(30))
}

fn device_id() -> DeviceId {
    DeviceId::from_uuid(Uuid::from_u128(32))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}

fn sha256(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

struct TestClock(Mutex<i64>);

impl TestClock {
    fn advance(&self, millis: i64) {
        *self.0.lock().unwrap() += millis;
    }
}

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        time(*self.0.lock().unwrap())
    }
}

/// 每次生成不同的“随机值”，摘要按真实的 SHA-256 算。
struct CountingSecrets(Mutex<u64>);

impl SecretFactory for CountingSecrets {
    fn generate(&self) -> Result<SecretValue, SecretGenerationFailure> {
        let mut counter = self.0.lock().unwrap();
        *counter += 1;
        Ok(SecretValue::new(format!("secret-{counter}")).unwrap())
    }

    fn digest(&self, value: &str) -> SecretDigest {
        SecretDigest::from_array(sha256(value))
    }
}

struct FixedRooms(PrivateRoomSnapshot);

impl PrivateRoomStore for FixedRooms {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("口令不建房间") })
    }
    fn find_by_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        let found = (catalog_id == self.0.catalog().id()).then(|| self.0.clone());
        Box::pin(async move { Ok(found) })
    }
    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("口令不列房间") })
    }
    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("口令按目录找房间") })
    }
    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("口令不改成员") })
    }
    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("口令不改名") })
    }
}

/// 这个 Agent 的主人是凭口令带它进来的人。
struct FixedMemberships;

impl AgentMembershipRepository for FixedMemberships {
    fn find_memberships(
        &self,
        agent: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentMemberships>>> {
        let found = (agent == agent_id())
            .then(|| AgentMemberships::with_initial_owner(agent, outsider(), time(NOW - 60_000)));
        Box::pin(async move { Ok(found) })
    }
}

#[derive(Default)]
struct RecordingMatrix(Mutex<Vec<&'static str>>);

impl RecordingMatrix {
    fn events(&self) -> Vec<&'static str> {
        self.0.lock().unwrap().clone()
    }
}

impl PrivateRoomMatrixGateway for RecordingMatrix {
    fn membership<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<Option<PrivateMatrixMembership>>> {
        Box::pin(async { unreachable!("移出不查成员状态") })
    }
    fn invite<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("兑换口令不邀请，入场时才邀请") })
    }
    fn kick<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        self.0.lock().unwrap().push("kick");
        Box::pin(async { Ok(()) })
    }
    fn ban<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("移出不封禁") })
    }
    fn set_speaking<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
        allowed: bool,
    ) -> PortFuture<'a, MatrixResult<()>> {
        self.0.lock().unwrap().push(if allowed {
            "speaking:true"
        } else {
            "speaking:false"
        });
        Box::pin(async { Ok(()) })
    }
    fn set_speaking_batch<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _assignments: &'a [PrivateMatrixSpeakingAssignment],
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!() })
    }
    fn archive<'a>(&'a self, _room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!() })
    }
    fn set_name<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _name: &'a str,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!() })
    }
}

/// 与 Postgres 实现同样的规则：一个房间一个口令，按摘要查找，猜错按固定窗口计数。
#[derive(Default)]
struct MemoryAccessStore {
    codes: Mutex<BTreeMap<RoomCatalogId, ([u8; 32], PrivateRoomJoinCodeRecord)>>,
    members: Mutex<BTreeMap<AgentId, PrivateRoomAgentMemberRecord>>,
    attempts: Mutex<BTreeMap<String, (UtcMillis, u32)>>,
}

impl MemoryAccessStore {
    fn has_digest(&self, code: &PrivateRoomJoinCode) -> bool {
        let digest = sha256(code.normalized());
        self.codes
            .lock()
            .unwrap()
            .values()
            .any(|(stored, _)| *stored == digest)
    }

    fn install_code(&self, code: PrivateRoomJoinCode) -> PrivateRoomJoinCode {
        self.codes.lock().unwrap().insert(
            catalog_id(),
            (
                sha256(code.normalized()),
                PrivateRoomJoinCodeRecord {
                    catalog_id: catalog_id(),
                    permissions: PrivateRoomPermissions::AGENT_MEMBER,
                    created_by: owner(),
                    created_at: time(NOW - 10_000),
                },
            ),
        );
        code
    }

    fn status(&self, agent_id: AgentId) -> Option<PrivateRoomAgentMemberStatus> {
        self.members
            .lock()
            .unwrap()
            .get(&agent_id)
            .map(|member| member.status)
    }

    fn failures(&self) -> u32 {
        self.attempts
            .lock()
            .unwrap()
            .values()
            .map(|(_, count)| *count)
            .sum()
    }

    fn callers(&self) -> Vec<String> {
        self.attempts.lock().unwrap().keys().cloned().collect()
    }
}

impl PrivateRoomAgentAccessStore for MemoryAccessStore {
    fn join_code(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomJoinCodeRecord>>> {
        let found = self
            .codes
            .lock()
            .unwrap()
            .get(&catalog_id)
            .map(|(_, record)| record.clone());
        Box::pin(async move { Ok(found) })
    }
    fn replace_join_code<'a>(
        &'a self,
        record: &'a PrivateRoomJoinCodeRecord,
        digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        self.codes
            .lock()
            .unwrap()
            .insert(record.catalog_id, (*digest.as_bytes(), record.clone()));
        Box::pin(async { Ok(()) })
    }
    fn clear_join_code(&self, catalog_id: RoomCatalogId) -> PortFuture<'_, RepositoryResult<bool>> {
        let removed = self.codes.lock().unwrap().remove(&catalog_id).is_some();
        Box::pin(async move { Ok(removed) })
    }
    fn find_join_code<'a>(
        &'a self,
        digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomJoinCodeRecord>>> {
        let found = self
            .codes
            .lock()
            .unwrap()
            .values()
            .find(|(stored, _)| stored == digest.as_bytes())
            .map(|(_, record)| record.clone());
        Box::pin(async move { Ok(found) })
    }
    fn agent_member(
        &self,
        _catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomAgentMemberRecord>>> {
        let found = self.members.lock().unwrap().get(&agent_id).cloned();
        Box::pin(async move { Ok(found) })
    }
    fn agent_members(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomAgentMemberRecord>>> {
        let all = self.members.lock().unwrap().values().cloned().collect();
        Box::pin(async move { Ok(all) })
    }
    fn admit_agent(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        permissions: PrivateRoomPermissions,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        let mut members = self.members.lock().unwrap();
        let joined_at = members
            .get(&agent_id)
            .filter(|member| member.status == PrivateRoomAgentMemberStatus::Joined)
            .map_or(now, |member| member.joined_at);
        members.insert(
            agent_id,
            PrivateRoomAgentMemberRecord {
                catalog_id,
                agent_id,
                display_name: "Scout".to_owned(),
                matrix_user_id: MatrixUserId::new("@_agent_scout:matrix.test").unwrap(),
                owner_display_name: Some("Outsider".to_owned()),
                status: PrivateRoomAgentMemberStatus::Joined,
                permissions,
                joined_at,
                status_changed_at: now,
            },
        );
        Box::pin(async { Ok(()) })
    }
    fn remove_agent(
        &self,
        _catalog_id: RoomCatalogId,
        agent_id: AgentId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        let removed = self
            .members
            .lock()
            .unwrap()
            .get_mut(&agent_id)
            .filter(|member| member.status == PrivateRoomAgentMemberStatus::Joined)
            .map(|member| {
                member.status = PrivateRoomAgentMemberStatus::Removed;
                member.status_changed_at = now;
            })
            .is_some();
        Box::pin(async move { Ok(removed) })
    }
    fn join_code_retry_at<'a>(
        &'a self,
        caller: &'a str,
        now: UtcMillis,
        policy: JoinCodeAttemptPolicy,
    ) -> PortFuture<'a, RepositoryResult<Option<UtcMillis>>> {
        let window = i64::try_from(policy.window.value()).unwrap();
        let retry_at = self
            .attempts
            .lock()
            .unwrap()
            .get(caller)
            .filter(|(started, count)| {
                *count >= policy.max_failures && now.value() < started.value() + window
            })
            .map(|(started, _)| time(started.value() + window));
        Box::pin(async move { Ok(retry_at) })
    }
    fn record_join_code_failure<'a>(
        &'a self,
        caller: &'a str,
        now: UtcMillis,
        policy: JoinCodeAttemptPolicy,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        let window = i64::try_from(policy.window.value()).unwrap();
        let mut attempts = self.attempts.lock().unwrap();
        let entry = attempts.entry(caller.to_owned()).or_insert((now, 0));
        if now.value() >= entry.0.value() + window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        Box::pin(async { Ok(()) })
    }
}
