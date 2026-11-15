//! 子房间第 2d-2 步（`specs/redesign/sub-rooms.md` 的“门”“进门”）：人申请进要批准的子房间、撤回，
//! 管得了这一间的人看、批准、拒绝。批准以后和他自己点“加入”一样直接记成已加入。
//!
//! 夹具是 `sub_rooms.rs` 里的那一份。申请和公开房间的申请同一张表，这里给内存存储补上那一套
//! （`PrivateRoomJoinRequestStore`）和锁着门记申请的那一步（`store_join_request`）；换完门不收申请的
//! 作废在 `revise_sub_room` 里，和真实存储一样。

use agent_room_application::{
    ports::{
        JoinRequestCreateOutcome, JoinRequestSweep, PendingJoinRequest, PrivateRoomJoinRequestStore,
    },
    private_rooms::{AnswerSubRoomJoinRequest, RequestSubRoomJoin, SubRoomJoinState},
};
use agent_room_domain::room_join_requests::{
    JOIN_REQUEST_REJECTION_COOLDOWN_MILLIS, JoinRequestStatus,
};

use super::{
    membership::{refusal, sub_room},
    *,
};

/// 申请的人在账户上的昵称写成了邮箱的样子：看申请的人只看到 @ 前面那段。
const REQUESTER_NAME: &str = "riley@example.test";

fn now() -> UtcMillis {
    UtcMillis::new(1_000).expect("时间有效")
}

fn inside(rooms: &Rooms, catalog: u128, person: u128) -> bool {
    rooms
        .store
        .room(catalog)
        .member(principal_id(person))
        .is_some_and(PrivateRoomMember::has_joined)
}

impl Rooms {
    async fn ask(
        &self,
        person: u128,
        catalog: u128,
        message: &str,
    ) -> Result<SubRoomJoinState, PrivateRoomFailure> {
        self.service
            .request_sub_room_join(RequestSubRoomJoin {
                actor: self::person(person),
                parent_catalog_id: catalog_id(PARENT),
                catalog_id: catalog_id(catalog),
                message: message.to_owned(),
            })
            .await
    }

    async fn withdraw(&self, person: u128, catalog: u128) -> Result<(), PrivateRoomFailure> {
        self.service
            .withdraw_sub_room_join_request(Self::membership(person, catalog))
            .await
    }

    async fn requests(
        &self,
        person: u128,
        catalog: u128,
    ) -> Result<Vec<PendingJoinRequest>, PrivateRoomFailure> {
        self.service
            .sub_room_join_requests(Self::membership(person, catalog))
            .await
    }

    async fn answer(
        &self,
        person: u128,
        catalog: u128,
        requester: u128,
        approve: bool,
    ) -> Result<JoinRequestStatus, PrivateRoomFailure> {
        self.service
            .answer_sub_room_join_request(AnswerSubRoomJoinRequest {
                actor: self::person(person),
                parent_catalog_id: catalog_id(PARENT),
                catalog_id: catalog_id(catalog),
                requester: principal_id(requester),
                approve,
            })
            .await
    }

    /// 一间要批准的子房间。
    fn seed_request_door(&self, catalog: u128, name: &str) {
        self.store.seed(&Seed {
            access: RoomJoinMode::Request,
            ..Seed::open(catalog, name)
        });
    }
}

/// 在等批准，申请的时间就是此刻。
fn pending() -> SubRoomJoinState {
    SubRoomJoinState::Pending {
        requested_at: now(),
    }
}

const SPEAKER: [PrivateRoomCapability; 3] = [
    PrivateRoomCapability::View,
    PrivateRoomCapability::Speak,
    PrivateRoomCapability::Automate,
];

#[tokio::test]
async fn 要批准的才记申请_没有门的和管这间房的直接进_只能请进来的和藏着的不收() {
    let rooms = Rooms::new();
    rooms.seed_request_door(20, "security");
    for (catalog, name, access, hidden) in [
        (21, "vault", RoomJoinMode::InviteOnly, false),
        (22, "release", RoomJoinMode::Open, false),
        (23, "secret", RoomJoinMode::Request, true),
    ] {
        rooms.store.seed(&Seed {
            access,
            hidden,
            ..Seed::open(catalog, name)
        });
    }

    assert_eq!(rooms.ask(MEMBER, 20, " 想帮忙看看 ").await, Ok(pending()));
    assert_eq!(rooms.events(), ["sub_rooms.request"], "只记申请，不进人");
    assert!(!inside(&rooms, 20, MEMBER));
    let stored = rooms.store.requests_of(20);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].message.as_str(), "想帮忙看看");
    assert_eq!(stored[0].status, JoinRequestStatus::Pending);

    // 不用批准的直接进：没有门的，父房间的管理员。
    assert_eq!(
        rooms.ask(MEMBER, 22, "").await,
        Ok(SubRoomJoinState::Joined)
    );
    assert!(inside(&rooms, 22, MEMBER));
    assert_eq!(rooms.ask(ADMIN, 20, "").await, Ok(SubRoomJoinState::Joined));
    assert!(inside(&rooms, 20, ADMIN), "房间里没有管理者进不去的地方");

    assert_eq!(
        refusal(rooms.ask(MEMBER, 21, "").await),
        sub_room(SubRoomRefusal::InviteOnly)
    );
    // 藏着的外面的人找不到；不在父房间里的人也一样。
    for (person, catalog) in [(MEMBER, 23), (OUTSIDER, 20)] {
        assert_eq!(
            refusal(rooms.ask(person, catalog, "").await),
            PrivateRoomFailureKind::NotFound
        );
    }
    assert!(rooms.store.requests_of(21).is_empty());
    assert!(rooms.store.requests_of(23).is_empty());
}

#[tokio::test]
async fn 同一个人只有一个在等的_留言不对的不收_撤回以后能再申请() {
    let rooms = Rooms::new();
    rooms.seed_request_door(20, "security");
    assert_eq!(rooms.ask(MEMBER, 20, "第一次").await, Ok(pending()));
    rooms.clear_events();

    assert_eq!(rooms.ask(MEMBER, 20, "第二次").await, Ok(pending()));
    assert!(rooms.events().is_empty(), "已经有一个在等的，原样回它");
    assert_eq!(rooms.store.requests_of(20)[0].message.as_str(), "第一次");
    assert_eq!(
        refusal(rooms.ask(VIEWER, 20, "第一行\r\n第二行").await),
        PrivateRoomFailureKind::InvalidRequest
    );
    let too_long = "字".repeat(201);
    assert_eq!(
        refusal(rooms.ask(VIEWER, 20, &too_long).await),
        PrivateRoomFailureKind::InvalidRequest
    );

    rooms.withdraw(MEMBER, 20).await.expect("能撤回");
    assert_eq!(
        rooms.store.requests_of(20)[0].status,
        JoinRequestStatus::Withdrawn
    );
    rooms
        .withdraw(MEMBER, 20)
        .await
        .expect("没有在等的也不算错");
    assert_eq!(rooms.ask(MEMBER, 20, "再来").await, Ok(pending()));
    assert_eq!(rooms.store.requests_of(20).len(), 2);

    // 地址里的房间对不上的，当没有这间子房间。
    let elsewhere = rooms
        .service
        .withdraw_sub_room_join_request(SubRoomMembershipAction {
            parent_catalog_id: catalog_id(99),
            ..Rooms::membership(MEMBER, 20)
        })
        .await;
    assert_eq!(refusal(elsewhere), PrivateRoomFailureKind::NotFound);
}

#[tokio::test]
async fn 被拒的24小时内不能再申请_回什么时候能再申请() {
    let rooms = Rooms::new();
    rooms.seed_request_door(20, "security");
    rooms.ask(MEMBER, 20, "").await.expect("能申请");
    assert_eq!(
        rooms.answer(OWNER, 20, MEMBER, false).await,
        Ok(JoinRequestStatus::Rejected)
    );
    assert!(!inside(&rooms, 20, MEMBER), "拒绝不进人");
    let stored = rooms.store.requests_of(20);
    assert_eq!(stored[0].decided_by, Some(principal_id(OWNER)));

    let again = rooms.ask(MEMBER, 20, "").await.expect_err("还不到 24 小时");
    assert_eq!(again.kind(), sub_room(SubRoomRefusal::RecentlyRejected));
    assert_eq!(
        again.retry_at(),
        Some(UtcMillis::new(1_000 + JOIN_REQUEST_REJECTION_COOLDOWN_MILLIS).expect("时间有效"))
    );
    assert_eq!(rooms.store.requests_of(20).len(), 1);
}

#[tokio::test]
async fn 管得了这一间的人才看得到申请_昵称只留前面那段_名单上写着几个在等() {
    let rooms = Rooms::new();
    rooms.seed_request_door(20, "security");
    rooms
        .service
        .create_sub_room(CreateSubRoom {
            access: RoomJoinMode::Request,
            ..Rooms::request(MEMBER, 21, "design")
        })
        .await
        .expect("成员建的，他管这一间");
    rooms.ask(MEMBER, 20, "").await.expect("能申请");
    rooms.ask(VIEWER, 21, "我也想看").await.expect("能申请");

    let seen = rooms.requests(MEMBER, 21).await.expect("这一间的管理员");
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].record.requester, principal_id(VIEWER));
    assert_eq!(seen[0].record.message.as_str(), "我也想看");
    assert_eq!(seen[0].requester_display_name, "riley");
    for person in [OWNER, ADMIN] {
        let seen = rooms
            .requests(person, 20)
            .await
            .expect("父房间的房主和管理员");
        assert_eq!(seen.len(), 1, "不用先进来");
    }
    assert_eq!(
        refusal(rooms.requests(VIEWER, 20).await),
        sub_room(SubRoomRefusal::ManagersOnly)
    );

    // 名单上：申请的人看到自己在等，管这一间的人看到有几个在等。
    let by_member = rooms.list(MEMBER, PARENT).await.expect("能读名单");
    let entry = |list: &agent_room_application::private_rooms::SubRoomList, catalog| {
        list.sub_rooms
            .iter()
            .find(|view| view.entry.catalog_id == catalog_id(catalog))
            .cloned()
            .expect("在名单里")
    };
    let security = entry(&by_member, 20);
    assert_eq!(security.entry.viewer_requested_at, Some(now()));
    assert!(!security.manages);
    assert!(entry(&by_member, 21).manages, "他建的那一间他管");
    let by_owner = rooms.list(OWNER, PARENT).await.expect("能读名单");
    let security = entry(&by_owner, 20);
    assert!(security.manages);
    assert_eq!(security.entry.pending_requests, 1);
    assert_eq!(security.entry.viewer_requested_at, None);
}

#[tokio::test]
async fn 批准_直接记成已加入再邀请_被禁言着的不给发言级别_申请记成批准() {
    let rooms = Rooms::new();
    rooms.seed_request_door(20, "security");
    rooms.ask(MEMBER, 20, "").await.expect("能申请");
    rooms.store.mute(PARENT, MEMBER);
    rooms.clear_events();

    assert_eq!(
        rooms.answer(OWNER, 20, MEMBER, true).await,
        Ok(JoinRequestStatus::Approved)
    );
    assert_eq!(
        rooms.events(),
        ["matrix.speak.off", "sub_rooms.admit", "matrix.invite"],
        "和他自己点“加入”同一段：发言级别、记成员、邀请"
    );
    let member = rooms.store.room(20);
    let member = member.member(principal_id(MEMBER)).expect("在里面");
    assert!(member.has_joined(), "不用再点接受");
    assert_eq!(member.permissions(), permissions(&SPEAKER));
    assert_eq!(
        rooms.matrix.speaking_allowed(matrix_user(MEMBER).as_str()),
        Some(false),
        "父房间里禁言着，进来也不给"
    );
    assert_eq!(
        rooms.matrix.membership_of(&matrix_user(MEMBER)),
        Some(PrivateMatrixMembership::Invited)
    );
    let stored = rooms.store.requests_of(20);
    assert_eq!(stored[0].status, JoinRequestStatus::Approved);
    assert_eq!(stored[0].decided_by, Some(principal_id(OWNER)));

    assert_eq!(
        refusal(rooms.answer(OWNER, 20, MEMBER, true).await),
        PrivateRoomFailureKind::NotFound,
        "已经处理过了"
    );
}

#[tokio::test]
async fn 批准不了的_封禁着的要先解封_不在父房间里的当没有这个申请() {
    let rooms = Rooms::new();
    rooms.seed_request_door(20, "security");
    rooms.ask(MEMBER, 20, "").await.expect("能申请");
    rooms.ask(VIEWER, 20, "").await.expect("能申请");
    rooms.store.change(20, |room| {
        room.admit(principal_id(MEMBER), permissions(&SPEAKER))
            .expect("进来过");
        room.ban_member(principal_id(OWNER), principal_id(MEMBER))
            .expect("房主能封禁");
    });
    rooms.store.change(PARENT, |parent| {
        parent
            .remove_member(principal_id(OWNER), principal_id(VIEWER))
            .expect("房主能移出");
    });
    rooms.clear_events();

    assert_eq!(
        refusal(rooms.answer(OWNER, 20, MEMBER, true).await),
        sub_room(SubRoomRefusal::TargetBanned)
    );
    assert_eq!(
        refusal(rooms.answer(OWNER, 20, VIEWER, true).await),
        PrivateRoomFailureKind::NotFound
    );
    assert!(rooms.events().is_empty(), "什么都没动");
    assert!(
        rooms
            .store
            .requests_of(20)
            .iter()
            .all(|request| request.status == JoinRequestStatus::Pending)
    );
}

#[tokio::test]
async fn 申请的当口门换了_这一次不记_回冲突() {
    let rooms = Rooms::new();
    rooms.seed_request_door(20, "security");
    rooms
        .store
        .close_door_before_request(RoomJoinMode::InviteOnly);

    assert_eq!(
        refusal(rooms.ask(MEMBER, 20, "").await),
        PrivateRoomFailureKind::Conflict
    );
    assert!(rooms.store.requests_of(20).is_empty());
}

#[tokio::test]
async fn 换完门不收申请的_在等的作废_改名不动它们() {
    let rooms = Rooms::new();
    rooms.seed_request_door(20, "security");
    rooms.seed_request_door(21, "design");
    rooms.ask(MEMBER, 20, "").await.expect("能申请");
    rooms.ask(MEMBER, 21, "").await.expect("能申请");

    rooms
        .revise(OWNER, 20, Some("security-2"), None)
        .await
        .expect("改名");
    rooms
        .change_door(OWNER, 21, None, Some(false))
        .await
        .expect("门没变，还收申请");
    assert_eq!(
        rooms.store.requests_of(20)[0].status,
        JoinRequestStatus::Pending
    );
    assert_eq!(
        rooms.store.requests_of(21)[0].status,
        JoinRequestStatus::Pending
    );

    rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::InviteOnly), None)
        .await
        .expect("改成只能请进来");
    rooms
        .change_door(OWNER, 21, None, Some(true))
        .await
        .expect("藏起来");
    for catalog in [20, 21] {
        let stored = rooms.store.requests_of(catalog);
        assert_eq!(stored[0].status, JoinRequestStatus::Voided);
        assert_eq!(stored[0].decided_by, None);
    }
}

#[tokio::test]
async fn 管不了的不能处理_归档了的不能处理() {
    let rooms = Rooms::new();
    rooms.seed_request_door(20, "security");
    rooms.ask(MEMBER, 20, "").await.expect("能申请");

    assert_eq!(
        refusal(rooms.answer(VIEWER, 20, MEMBER, true).await),
        sub_room(SubRoomRefusal::ManagersOnly)
    );
    rooms.store.archive(20);
    assert_eq!(
        refusal(rooms.answer(OWNER, 20, MEMBER, false).await),
        sub_room(SubRoomRefusal::Archived)
    );
    assert_eq!(
        rooms.store.requests_of(20)[0].status,
        JoinRequestStatus::Pending
    );
}

// ---- 夹具：申请

impl MemoryRooms {
    /// 锁着门记申请，照真实存储的规则：他还在父房间里、这一间此刻收申请才记，同一个人只有一个在等的。
    pub(super) fn store_join_request(
        &self,
        parent: RoomCatalogId,
        record: &JoinRequestRecord,
    ) -> SubRoomJoinRequestOutcome {
        self.record("sub_rooms.request");
        if let Some(access) = self
            .door_moves_before_request
            .lock()
            .expect("门锁正常")
            .take()
        {
            for stored in self.links.lock().expect("关系锁正常").iter_mut() {
                if stored.link.catalog_id == record.catalog_id {
                    stored.link.access = access;
                }
            }
        }
        let in_parent = self.rooms.lock().expect("房间锁正常")[&parent]
            .room()
            .member(record.requester)
            .is_some_and(PrivateRoomMember::has_joined);
        if !in_parent {
            return SubRoomJoinRequestOutcome::NotInParent;
        }
        let takes = self.links.lock().expect("关系锁正常").iter().any(|stored| {
            stored.link.catalog_id == record.catalog_id
                && stored.link.access == RoomJoinMode::Request
                && !stored.link.hidden
                && !stored.archived
        });
        if !takes {
            return SubRoomJoinRequestOutcome::Closed;
        }
        if let Some(existing) = self.pending_request(record.catalog_id, record.requester) {
            return SubRoomJoinRequestOutcome::AlreadyPending(existing.requested_at);
        }
        self.requests
            .lock()
            .expect("申请锁正常")
            .push(record.clone());
        SubRoomJoinRequestOutcome::Created
    }

    pub(super) fn pending_request(
        &self,
        catalog: RoomCatalogId,
        requester: PrincipalId,
    ) -> Option<JoinRequestRecord> {
        self.requests
            .lock()
            .expect("申请锁正常")
            .iter()
            .find(|request| {
                request.catalog_id == catalog
                    && request.requester == requester
                    && request.status == JoinRequestStatus::Pending
            })
            .cloned()
    }

    pub(super) fn pending_count(&self, catalog: RoomCatalogId) -> u32 {
        let count = self
            .requests
            .lock()
            .expect("申请锁正常")
            .iter()
            .filter(|request| {
                request.catalog_id == catalog && request.status == JoinRequestStatus::Pending
            })
            .count();
        u32::try_from(count).expect("申请不多")
    }

    pub(super) fn void_requests(&self, catalog: RoomCatalogId, at: UtcMillis) {
        for request in self.requests.lock().expect("申请锁正常").iter_mut() {
            if request.catalog_id == catalog && request.status == JoinRequestStatus::Pending {
                request.status = JoinRequestStatus::Voided;
                request.decided_at = Some(at);
            }
        }
    }

    /// 存子房间时，在这一间里被封禁的人在等的申请作废（2d-3），和真实存储一样。
    pub(super) fn void_banned_requests(&self, room: &PrivateRoom) {
        let is_sub_room = self
            .links
            .lock()
            .expect("关系锁正常")
            .iter()
            .any(|stored| stored.link.catalog_id == room.catalog_id());
        if !is_sub_room {
            return;
        }
        for request in self.requests.lock().expect("申请锁正常").iter_mut() {
            let banned = room
                .member(request.requester)
                .is_some_and(|member| member.status() == PrivateRoomMembershipStatus::Banned);
            if request.catalog_id == room.catalog_id()
                && request.status == JoinRequestStatus::Pending
                && banned
            {
                request.status = JoinRequestStatus::Voided;
                request.decided_at = Some(request.requested_at);
            }
        }
    }

    pub(super) fn requests_of(&self, catalog: u128) -> Vec<JoinRequestRecord> {
        self.requests
            .lock()
            .expect("申请锁正常")
            .iter()
            .filter(|request| request.catalog_id == catalog_id(catalog))
            .cloned()
            .collect()
    }

    /// 下一次记申请之前，另一个请求先把门换成 `access`。
    fn close_door_before_request(&self, access: RoomJoinMode) {
        *self.door_moves_before_request.lock().expect("门锁正常") = Some(access);
    }
}

impl PrivateRoomJoinRequestStore for MemoryRooms {
    fn pending_join_request(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<JoinRequestRecord>>> {
        let pending = self.pending_request(catalog_id, requester);
        Box::pin(async move { Ok(pending) })
    }

    fn last_rejected_at(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<UtcMillis>>> {
        let rejected = self
            .requests
            .lock()
            .expect("申请锁正常")
            .iter()
            .filter(|request| {
                request.catalog_id == catalog_id
                    && request.requester == requester
                    && request.status == JoinRequestStatus::Rejected
            })
            .filter_map(|request| request.decided_at)
            .max();
        Box::pin(async move { Ok(rejected) })
    }

    fn create_join_request<'a>(
        &'a self,
        _record: &'a JoinRequestRecord,
    ) -> PortFuture<'a, RepositoryResult<JoinRequestCreateOutcome>> {
        unreachable!("子房间的申请锁着门记，走 request_sub_room_join")
    }

    fn pending_join_requests(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PendingJoinRequest>>> {
        let pending = self
            .requests
            .lock()
            .expect("申请锁正常")
            .iter()
            .filter(|request| {
                request.catalog_id == catalog_id && request.status == JoinRequestStatus::Pending
            })
            .map(|request| PendingJoinRequest {
                record: request.clone(),
                requester_display_name: REQUESTER_NAME.to_owned(),
            })
            .collect();
        Box::pin(async move { Ok(pending) })
    }

    fn decide_join_request(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
        status: JoinRequestStatus,
        decided_by: Option<PrincipalId>,
        decided_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        let mut requests = self.requests.lock().expect("申请锁正常");
        let decided = requests.iter_mut().find(|request| {
            request.catalog_id == catalog_id
                && request.requester == requester
                && request.status == JoinRequestStatus::Pending
        });
        let found = decided.is_some();
        if let Some(request) = decided {
            request.status = status;
            request.decided_by = decided_by;
            request.decided_at = Some(decided_at);
        }
        Box::pin(async move { Ok(found) })
    }

    fn void_pending_join_requests(
        &self,
        _catalog_id: RoomCatalogId,
        _voided_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<u64>> {
        unreachable!("只有公开房间改回私密、平台拿掉时用")
    }

    fn sweep_join_requests(
        &self,
        _now: UtcMillis,
        _decided_before: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<JoinRequestSweep>> {
        unreachable!("定时清理不在这些用例里")
    }
}
