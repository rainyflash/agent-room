//! 私人房间的邀请链接（`specs/room-people/design.md`）：有邀请权限的成员生成、再看、停用；拿到链接的
//! 人先看一眼，再用它进来；链接过期、停用、创建人没了邀请权限、房间归档就不管用；猜错按账号限流。

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        Clock, JoinCodeAttemptPolicy, JoinCodeSealer, PortFuture, PrivateRoomInviteLinkRecord,
        PrivateRoomInviteLinkStore, PrivateRoomSnapshot, PrivateRoomStore, RoomAgentOwnerRecord,
        RoomPeopleDirectory, RoomPersonName, SealedInviteLink, SealedSecret, SecretDigest,
        SecretFactory, SecretGenerationFailure, SecretSealingFailure, SecretValue,
    },
    private_rooms::{
        CreateInviteLink, InviteByLink, InviteLinkFailureKind, InviteeMembership, ManageInviteLink,
        PrivateRoomFailure, PrivateRoomFailureKind, PrivateRoomFailureStage,
        PrivateRoomInviteLinkDependencies, PrivateRoomInviteLinkService,
        PrivateRoomInviteLinkUseCases, PrivateRoomLinkInvitations, PrivateRoomResult,
        UseInviteLink,
    },
};
use agent_room_domain::{
    DomainError,
    ids::{AgentId, PrincipalId, RoomCatalogId, RoomInstanceId},
    private_rooms::{
        PrivateRoom, PrivateRoomCapability, PrivateRoomLifecycleStatus, PrivateRoomMember,
        PrivateRoomMembershipStatus, PrivateRoomPermissions,
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
const DAY: i64 = 24 * 3_600_000;

#[tokio::test]
async fn 房主生成链接_默认给查看发言和自动发言_七天过期_再生成就换掉旧的() {
    let fixture = Fixture::new(room_with_members());

    let first = fixture
        .service
        .create(create(owner(), None))
        .await
        .expect("房主能生成");
    assert_eq!(first.record.permissions, automate_permissions());
    assert_eq!(first.record.created_by, owner());
    assert_eq!(first.record.expires_at, time(NOW + 7 * DAY));
    let revealed = fixture
        .service
        .reveal(manage(owner()))
        .await
        .expect("能再看");
    assert_eq!(revealed.secret, first.secret);

    let second = fixture
        .service
        .create(create(owner(), None))
        .await
        .expect("再生成一条");
    assert_ne!(second.secret, first.secret);
    assert_eq!(
        fixture
            .service
            .preview(use_link(newcomer(), first.secret.as_str()))
            .await
            .expect_err("旧链接不管用了")
            .kind(),
        InviteLinkFailureKind::NotFound
    );
    assert_eq!(
        fixture
            .service
            .reveal(manage(owner()))
            .await
            .expect("能再看")
            .secret,
        second.secret
    );
}

#[tokio::test]
async fn 只有已加入且能邀请的人管链接_给不出自己没有的权限() {
    let fixture = Fixture::new(room_with_members());
    let failure = |actor, permissions| {
        let service = &fixture.service;
        async move {
            service
                .create(create(actor, permissions))
                .await
                .expect_err("不能生成")
                .kind()
        }
    };

    assert_eq!(
        failure(speaker(), None).await,
        InviteLinkFailureKind::Forbidden
    );
    assert_eq!(
        failure(newcomer(), None).await,
        InviteLinkFailureKind::NotFound,
        "不在房间里的和没有这个房间一样"
    );
    assert_eq!(
        failure(inviter(), Some(automate_permissions())).await,
        InviteLinkFailureKind::Forbidden,
        "自己没有自动发言就给不出"
    );
    let link = fixture
        .service
        .create(create(inviter(), None))
        .await
        .expect("能邀请的成员能生成");
    assert_eq!(
        link.record.permissions,
        speak_permissions(),
        "默认的不超过自己有的"
    );
    assert_eq!(
        fixture
            .service
            .disable(manage(speaker()))
            .await
            .expect_err("不能停用")
            .kind(),
        InviteLinkFailureKind::Forbidden
    );
}

#[tokio::test]
async fn 拿着链接的人先看到房间和邀请人_用它进来成了受邀_再用不重复邀请() {
    let fixture = Fixture::new(room_with_members());
    let link = fixture
        .service
        .create(create(inviter(), None))
        .await
        .expect("能生成");

    let preview = fixture
        .service
        .preview(use_link(newcomer(), link.secret.as_str()))
        .await
        .expect("能看");
    assert_eq!(preview.catalog_id, catalog_id());
    assert_eq!(preview.room_name, "项目室");
    assert_eq!(
        preview.inviter_display_name, "xiaoming",
        "名字里的邮箱只显示 @ 前面"
    );
    assert_eq!(preview.permissions, speak_permissions());
    assert_eq!(preview.membership, InviteeMembership::Absent);

    // 网页给的链接可能是大写或带着空白：照样认。
    let shouted = format!(" {} ", link.secret.as_str().to_ascii_uppercase());
    let redeemed = fixture
        .service
        .redeem(use_link(newcomer(), &shouted))
        .await
        .expect("能进来");
    assert_eq!(redeemed.membership, InviteeMembership::Invited);
    assert_eq!(
        fixture.invitations(),
        vec![InviteByLink {
            catalog_id: catalog_id(),
            link_creator: inviter(),
            target_principal_id: newcomer(),
            permissions: speak_permissions(),
        }]
    );
    assert_eq!(
        fixture.status_of(newcomer()),
        Some(PrivateRoomMembershipStatus::Invited)
    );

    let again = fixture
        .service
        .redeem(use_link(newcomer(), link.secret.as_str()))
        .await
        .expect("再用一次");
    assert_eq!(again.membership, InviteeMembership::Invited);
    assert_eq!(fixture.invitations().len(), 1, "已经受邀的不再邀请");
}

#[tokio::test]
async fn 已经在房间里的直接打开_被封禁的进不来_被移出的能再进() {
    let mut room = room_with_members();
    room.ban_member(owner(), banned()).unwrap();
    join(&mut room, removed());
    room.remove_member(owner(), removed()).unwrap();
    let fixture = Fixture::new(room);
    let link = fixture
        .service
        .create(create(owner(), None))
        .await
        .expect("能生成");

    let joined = fixture
        .service
        .redeem(use_link(speaker(), link.secret.as_str()))
        .await
        .expect("已经在房间里");
    assert_eq!(joined.membership, InviteeMembership::Joined);
    for result in [
        fixture
            .service
            .preview(use_link(banned(), link.secret.as_str()))
            .await
            .map(|_| ()),
        fixture
            .service
            .redeem(use_link(banned(), link.secret.as_str()))
            .await
            .map(|_| ()),
    ] {
        assert_eq!(
            result.expect_err("被封禁").kind(),
            InviteLinkFailureKind::Banned
        );
    }
    let back = fixture
        .service
        .redeem(use_link(removed(), link.secret.as_str()))
        .await
        .expect("被移出的能再进");
    assert_eq!(back.membership, InviteeMembership::Invited);
    assert_eq!(fixture.invitations().len(), 1);
}

#[tokio::test]
async fn 过期_停用_创建人没了邀请权限_房间归档_链接都不管用() {
    let fixture = Fixture::new(room_with_members());
    let expired = fixture.service.create(create(owner(), None)).await.unwrap();
    fixture.clock.advance(7 * DAY);
    fixture.assert_dead(expired.secret.as_str()).await;
    assert_eq!(
        fixture
            .service
            .reveal(manage(owner()))
            .await
            .unwrap_err()
            .kind(),
        InviteLinkFailureKind::NotFound,
        "过期的当作没有"
    );

    let disabled = fixture.service.create(create(owner(), None)).await.unwrap();
    fixture.service.disable(manage(owner())).await.unwrap();
    fixture.assert_dead(disabled.secret.as_str()).await;

    let inviters = fixture
        .service
        .create(create(inviter(), None))
        .await
        .unwrap();
    fixture.update_room(|room| {
        room.update_permissions(owner(), inviter(), speak_permissions())
            .map(|_| ())
    });
    fixture.assert_dead(inviters.secret.as_str()).await;

    let owners = fixture.service.create(create(owner(), None)).await.unwrap();
    fixture.update_room(|room| room.archive(owner()).map(|_| ()));
    fixture.assert_dead(owners.secret.as_str()).await;
    assert!(fixture.invitations().is_empty());
}

#[tokio::test]
async fn 猜错按账号计数_格式不对不算_到上限以后要等窗口过去() {
    let fixture = Fixture::new(room_with_members());
    let link = fixture.service.create(create(owner(), None)).await.unwrap();
    let wrong = "0".repeat(26);

    for _ in 0..3 {
        assert_eq!(
            fixture
                .service
                .preview(use_link(newcomer(), "not-a-link"))
                .await
                .unwrap_err()
                .kind(),
            InviteLinkFailureKind::InvalidRequest
        );
    }
    for _ in 0..MAX_FAILURES {
        assert_eq!(
            fixture
                .service
                .redeem(use_link(newcomer(), &wrong))
                .await
                .unwrap_err()
                .kind(),
            InviteLinkFailureKind::NotFound
        );
    }
    let limited = fixture
        .service
        .preview(use_link(newcomer(), link.secret.as_str()))
        .await
        .unwrap_err();
    assert_eq!(limited.kind(), InviteLinkFailureKind::RateLimited);
    assert_eq!(limited.retry_at(), Some(time(NOW + WINDOW)));
    fixture
        .service
        .preview(use_link(removed(), link.secret.as_str()))
        .await
        .expect("别的账号不受影响");

    fixture.clock.advance(WINDOW);
    fixture
        .service
        .preview(use_link(newcomer(), link.secret.as_str()))
        .await
        .expect("窗口过后恢复");
}

#[tokio::test]
async fn 封存不了时照样生成_以后看不到_登录过期的一律不行() {
    let fixture = Fixture::new(room_with_members());
    *fixture.sealer.unavailable.lock().unwrap() = true;
    let link = fixture
        .service
        .create(create(owner(), None))
        .await
        .expect("照样生成");
    assert_eq!(
        fixture
            .service
            .reveal(manage(owner()))
            .await
            .unwrap_err()
            .kind(),
        InviteLinkFailureKind::NotRevealable
    );
    fixture
        .service
        .preview(use_link(newcomer(), link.secret.as_str()))
        .await
        .expect("链接照样能用");

    let mut expired = principal(newcomer());
    expired.expires_at = time(NOW);
    assert_eq!(
        fixture
            .service
            .redeem(UseInviteLink {
                actor: expired,
                secret: link.secret.as_str().to_owned(),
            })
            .await
            .unwrap_err()
            .kind(),
        InviteLinkFailureKind::Forbidden
    );
}

const MAX_FAILURES: u32 = 4;
const WINDOW: i64 = 3_600_000;

struct Fixture {
    service: PrivateRoomInviteLinkService,
    rooms: Arc<MemoryRooms>,
    invitations: Arc<RoomInvitations>,
    clock: Arc<TestClock>,
    sealer: Arc<FakeSealer>,
}

impl Fixture {
    fn new(room: PrivateRoom) -> Self {
        let rooms = Arc::new(MemoryRooms(Mutex::new(room)));
        let invitations = Arc::new(RoomInvitations {
            rooms: rooms.clone(),
            calls: Mutex::default(),
        });
        let clock = Arc::new(TestClock(Mutex::new(NOW)));
        let sealer = Arc::new(FakeSealer::default());
        let service = PrivateRoomInviteLinkService::new(PrivateRoomInviteLinkDependencies {
            rooms: rooms.clone(),
            links: Arc::new(MemoryLinks::default()),
            invitations: invitations.clone(),
            people: Arc::new(Names),
            secrets: Arc::new(CountingSecrets(Mutex::new(0))),
            sealer: sealer.clone(),
            clock: clock.clone(),
            attempts: JoinCodeAttemptPolicy {
                window: DurationMillis::new(u64::try_from(WINDOW).unwrap()).unwrap(),
                max_failures: MAX_FAILURES,
            },
        });
        Self {
            service,
            rooms,
            invitations,
            clock,
            sealer,
        }
    }

    fn invitations(&self) -> Vec<InviteByLink> {
        self.invitations.calls.lock().unwrap().clone()
    }

    fn status_of(&self, principal_id: PrincipalId) -> Option<PrivateRoomMembershipStatus> {
        self.rooms
            .0
            .lock()
            .unwrap()
            .member(principal_id)
            .map(PrivateRoomMember::status)
    }

    fn update_room(&self, change: impl FnOnce(&mut PrivateRoom) -> Result<(), DomainError>) {
        change(&mut self.rooms.0.lock().unwrap()).expect("能改房间");
    }

    async fn assert_dead(&self, secret: &str) {
        for result in [
            self.service
                .preview(use_link(newcomer(), secret))
                .await
                .map(|_| ()),
            self.service
                .redeem(use_link(newcomer(), secret))
                .await
                .map(|_| ()),
        ] {
            assert_eq!(
                result.expect_err("链接不管用了").kind(),
                InviteLinkFailureKind::NotFound
            );
        }
    }
}

fn create(actor: PrincipalId, permissions: Option<PrivateRoomPermissions>) -> CreateInviteLink {
    CreateInviteLink {
        actor: principal(actor),
        catalog_id: catalog_id(),
        permissions,
    }
}

fn manage(actor: PrincipalId) -> ManageInviteLink {
    ManageInviteLink {
        actor: principal(actor),
        catalog_id: catalog_id(),
    }
}

fn use_link(actor: PrincipalId, secret: &str) -> UseInviteLink {
    UseInviteLink {
        actor: principal(actor),
        secret: secret.to_owned(),
    }
}

/// 房主、只能发言的成员、能邀请（没有自动发言）的成员。
fn room_with_members() -> PrivateRoom {
    let mut room = PrivateRoom::create(catalog_id(), owner());
    join(&mut room, speaker());
    room.invite(
        owner(),
        inviter(),
        PrivateRoomPermissions::from_capabilities([
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Invite,
        ])
        .unwrap(),
    )
    .unwrap();
    room.accept_invitation(inviter()).unwrap();
    room
}

fn join(room: &mut PrivateRoom, principal_id: PrincipalId) {
    room.invite(owner(), principal_id, speak_permissions())
        .unwrap();
    room.accept_invitation(principal_id).unwrap();
}

fn speak_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
    ])
    .unwrap()
}

fn automate_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::INVITED_BY_AGENT
}

fn snapshot(room: PrivateRoom) -> PrivateRoomSnapshot {
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
            matrix_room_id: MatrixRoomReference::new("!project:matrix.test".to_owned()).unwrap(),
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

fn principal(principal_id: PrincipalId) -> AuthenticatedPrincipal {
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

fn person(sequence: u128) -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(sequence))
}

fn owner() -> PrincipalId {
    person(1)
}

fn speaker() -> PrincipalId {
    person(2)
}

/// 能邀请、没有自动发言的成员，名字是邮箱。
fn inviter() -> PrincipalId {
    person(3)
}

/// 拿到链接的人，还不在房间里。
fn newcomer() -> PrincipalId {
    person(4)
}

fn banned() -> PrincipalId {
    person(5)
}

fn removed() -> PrincipalId {
    person(6)
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(10))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
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
        SecretDigest::from_array(Sha256::digest(value.as_bytes()).into())
    }
}

/// 封存替身：把房间号和打乱过的明文拼在一起；可以设成没配封存密钥。
#[derive(Default)]
struct FakeSealer {
    unavailable: Mutex<bool>,
}

impl JoinCodeSealer for FakeSealer {
    fn seal(
        &self,
        catalog_id: RoomCatalogId,
        plaintext: &[u8],
    ) -> Result<SealedSecret, SecretSealingFailure> {
        if *self.unavailable.lock().unwrap() {
            return Err(SecretSealingFailure::Unavailable);
        }
        let mut bytes = catalog_id.as_uuid().as_bytes().to_vec();
        bytes.extend(plaintext.iter().map(|byte| byte ^ 0x5A));
        Ok(SealedSecret {
            key_version: 1,
            bytes,
        })
    }

    fn open(
        &self,
        catalog_id: RoomCatalogId,
        sealed: &SealedSecret,
    ) -> Result<Vec<u8>, SecretSealingFailure> {
        let (room, body) = sealed.bytes.split_at(16);
        if room != catalog_id.as_uuid().as_bytes() {
            return Err(SecretSealingFailure::Corrupt);
        }
        Ok(body.iter().map(|byte| byte ^ 0x5A).collect())
    }
}

/// 一个房间，邀请替身直接改它。
struct MemoryRooms(Mutex<PrivateRoom>);

impl PrivateRoomStore for MemoryRooms {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("链接不建房间") })
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
        Box::pin(async { unreachable!("链接不列房间") })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("链接按目录找房间") })
    }

    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("链接本身不改成员") })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("链接不改名") })
    }
}

/// 照私人房间服务的规则以创建人的名义邀请：创建人还能管链接，权限不超过他的。
struct RoomInvitations {
    rooms: Arc<MemoryRooms>,
    calls: Mutex<Vec<InviteByLink>>,
}

impl PrivateRoomLinkInvitations for RoomInvitations {
    fn invite_by_link(
        &self,
        request: InviteByLink,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.calls.lock().unwrap().push(request.clone());
        let mut room = self.rooms.0.lock().unwrap();
        let result = room
            .authorize_invite_link(request.link_creator)
            .and_then(|()| {
                room.invite(
                    request.link_creator,
                    request.target_principal_id,
                    request.permissions,
                )
            })
            .map(|_| snapshot(room.clone()))
            .map_err(|_| {
                PrivateRoomFailure::new(
                    "test.invite_by_link",
                    PrivateRoomFailureStage::Validation,
                    PrivateRoomFailureKind::Forbidden,
                )
            });
        Box::pin(async move { result })
    }
}

#[derive(Default)]
struct MemoryLinks {
    link: Mutex<Option<SealedInviteLink>>,
    attempts: Mutex<BTreeMap<String, (i64, u32)>>,
}

impl PrivateRoomInviteLinkStore for MemoryLinks {
    fn replace_invite_link<'a>(
        &'a self,
        record: &'a PrivateRoomInviteLinkRecord,
        digest: &'a SecretDigest,
        sealed: Option<&'a SealedSecret>,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        *self.link.lock().unwrap() = Some(SealedInviteLink {
            record: record.clone(),
            digest: *digest,
            sealed: sealed.cloned(),
        });
        Box::pin(async { Ok(()) })
    }

    fn invite_link(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<SealedInviteLink>>> {
        let link = self
            .link
            .lock()
            .unwrap()
            .clone()
            .filter(|link| link.record.catalog_id == catalog_id);
        Box::pin(async move { Ok(link) })
    }

    fn find_invite_link<'a>(
        &'a self,
        digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomInviteLinkRecord>>> {
        let record = self
            .link
            .lock()
            .unwrap()
            .clone()
            .filter(|link| &link.digest == digest)
            .map(|link| link.record);
        Box::pin(async move { Ok(record) })
    }

    fn clear_invite_link(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        let mut link = self.link.lock().unwrap();
        let cleared = link
            .as_ref()
            .is_some_and(|link| link.record.catalog_id == catalog_id);
        if cleared {
            *link = None;
        }
        Box::pin(async move { Ok(cleared) })
    }

    fn invite_link_retry_at<'a>(
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
            .filter(|(started, failures)| {
                *failures >= policy.max_failures && now.value() < started + window
            })
            .map(|(started, _)| time(started + window));
        Box::pin(async move { Ok(retry_at) })
    }

    fn record_invite_link_failure<'a>(
        &'a self,
        caller: &'a str,
        now: UtcMillis,
        policy: JoinCodeAttemptPolicy,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        let window = i64::try_from(policy.window.value()).unwrap();
        let mut attempts = self.attempts.lock().unwrap();
        let entry = attempts
            .entry(caller.to_owned())
            .or_insert((now.value(), 0));
        if entry.0 + window <= now.value() {
            *entry = (now.value(), 0);
        }
        entry.1 += 1;
        Box::pin(async { Ok(()) })
    }
}

/// 房主叫“房主”，能邀请的成员注册时没填昵称，名字是邮箱。
struct Names;

impl RoomPeopleDirectory for Names {
    fn member_names(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomPersonName>>> {
        Box::pin(async {
            Ok(vec![
                RoomPersonName {
                    principal_id: owner(),
                    display_name: "房主".to_owned(),
                },
                RoomPersonName {
                    principal_id: inviter(),
                    display_name: "xiaoming@example.com".to_owned(),
                },
            ])
        })
    }

    fn agent_owners<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _agent_ids: &'a [AgentId],
    ) -> PortFuture<'a, RepositoryResult<Vec<RoomAgentOwnerRecord>>> {
        Box::pin(async { unreachable!("链接不问 Agent 的主人") })
    }
}
