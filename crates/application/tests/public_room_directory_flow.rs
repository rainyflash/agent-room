//! 发现页（`specs/public-rooms/design.md`）：搜索的字要像样；多拿一间看有没有下一页；“一小时内有人
//! 说话”按此刻往前一小时；昵称里像邮箱的只留 @ 前面；只有登录着的人才查自己的处境。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        Clock, MatrixResult, MatrixUserId, PortFuture, PublicRoomDetailRecord,
        PublicRoomDirectoryStore, PublicRoomPresenceMatrix, PublicRoomRecord, PublicRoomSearch,
        PublicRoomSort, PublicRoomViewerState,
    },
    public_room_directory::{
        BrowsePublicRooms, MAX_PUBLIC_ROOM_OFFSET, PUBLIC_ROOM_PAGE_SIZE,
        PublicDirectoryFailureKind, PublicRoomDirectoryDependencies, PublicRoomDirectoryService,
        PublicRoomDirectoryUseCases, PublicRoomOnline, RECENTLY_ACTIVE_MILLIS,
    },
};
use agent_room_domain::{
    agent_lifecycle::MatrixPresenceState,
    ids::{PrincipalId, RoomCatalogId},
    room_listing::{RoomJoinMode, RoomShareCode},
    time::UtcMillis,
};
use uuid::Uuid;

const NOW: i64 = 1_790_000_000_000;

#[tokio::test]
async fn 搜索的字去掉首尾空白_太长的带控制字符的翻太深的不收() {
    let fixture = Fixture::new(3);
    fixture
        .service
        .browse(BrowsePublicRooms {
            text: "  读书  ".to_owned(),
            join_mode: Some(RoomJoinMode::Request),
            recently_active: true,
            unencrypted_only: true,
            sort: PublicRoomSort::Newest,
            offset: 24,
            ..browse(None)
        })
        .await
        .expect("能翻");
    fixture
        .service
        .browse(browse(None))
        .await
        .expect("不搜也行");
    assert_eq!(
        *fixture.store.searches.lock().unwrap(),
        [
            PublicRoomSearch {
                text: Some("读书".to_owned()),
                join_mode: Some(RoomJoinMode::Request),
                active_since: Some(time(NOW - RECENTLY_ACTIVE_MILLIS)),
                unencrypted_only: true,
                sort: PublicRoomSort::Newest,
                offset: 24,
                limit: PUBLIC_ROOM_PAGE_SIZE + 1,
            },
            PublicRoomSearch {
                text: None,
                join_mode: None,
                active_since: None,
                unencrypted_only: false,
                sort: PublicRoomSort::Active,
                offset: 0,
                limit: PUBLIC_ROOM_PAGE_SIZE + 1,
            },
        ]
    );

    let too_long = "字".repeat(101);
    for request in [
        BrowsePublicRooms {
            text: too_long,
            ..browse(None)
        },
        BrowsePublicRooms {
            text: "读\u{0}书".to_owned(),
            ..browse(None)
        },
        BrowsePublicRooms {
            offset: MAX_PUBLIC_ROOM_OFFSET + 1,
            ..browse(None)
        },
    ] {
        let failure = fixture.service.browse(request).await.unwrap_err();
        assert_eq!(failure.kind(), PublicDirectoryFailureKind::InvalidRequest);
    }
    assert_eq!(fixture.store.searches.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn 多拿一间看有没有下一页_翻到头就不再给() {
    let full = Fixture::new(PUBLIC_ROOM_PAGE_SIZE as usize + 1);
    let page = full.service.browse(browse(None)).await.unwrap();
    assert_eq!(
        page.rooms.len(),
        PUBLIC_ROOM_PAGE_SIZE as usize,
        "多拿的那间不给"
    );
    assert_eq!(page.next_offset, Some(PUBLIC_ROOM_PAGE_SIZE));

    let last = full
        .service
        .browse(BrowsePublicRooms {
            offset: MAX_PUBLIC_ROOM_OFFSET,
            ..browse(None)
        })
        .await
        .unwrap();
    assert_eq!(last.next_offset, None, "再往后翻不收");

    let short = Fixture::new(PUBLIC_ROOM_PAGE_SIZE as usize);
    let page = short.service.browse(browse(None)).await.unwrap();
    assert_eq!(page.rooms.len(), PUBLIC_ROOM_PAGE_SIZE as usize);
    assert_eq!(page.next_offset, None);
}

#[tokio::test]
async fn 房主的昵称只留_at_前面_登录着的人才带上自己的处境() {
    let fixture = Fixture::new(2);
    let anonymous = fixture.service.browse(browse(None)).await.unwrap();
    assert_eq!(anonymous.rooms[0].room.owner_display_name, "aming");
    assert!(anonymous.rooms.iter().all(|room| room.viewer.is_none()));
    assert!(fixture.store.viewer_lookups.lock().unwrap().is_empty());

    let page = fixture
        .service
        .browse(browse(Some(viewer(NOW + 60_000))))
        .await
        .unwrap();
    assert_eq!(page.rooms[0].viewer, Some(PublicRoomViewerState::Pending));
    assert_eq!(page.rooms[1].viewer, None);
    assert_eq!(
        *fixture.store.viewer_lookups.lock().unwrap(),
        [(viewer_id(), vec![room_id(0), room_id(1)])]
    );

    // 登录刚过期：当成没登录，不去查。
    let expired = fixture
        .service
        .browse(browse(Some(viewer(NOW))))
        .await
        .unwrap();
    assert!(expired.rooms.iter().all(|room| room.viewer.is_none()));
    assert_eq!(fixture.store.viewer_lookups.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn 分享码对得上才有详情_管理员的昵称也处理过() {
    let fixture = Fixture::new(1);
    let detail = fixture
        .service
        .detail(Some(viewer(NOW + 60_000)), "Room00".to_owned())
        .await
        .expect("有这间房");
    assert_eq!(detail.room.room.catalog_id, room_id(0));
    assert_eq!(detail.room.viewer, Some(PublicRoomViewerState::Pending));
    assert_eq!(detail.admin_display_names, ["xiaolin", "老王"]);
    assert_eq!(
        detail.online,
        Some(PublicRoomOnline {
            count: 1,
            at_least: false,
        })
    );

    for code in ["Gone99", "bad code", ""] {
        let failure = fixture
            .service
            .detail(None, code.to_owned())
            .await
            .unwrap_err();
        assert_eq!(
            failure.kind(),
            PublicDirectoryFailureKind::NotFound,
            "{code}"
        );
    }
    assert_eq!(
        *fixture.store.codes.lock().unwrap(),
        ["Room00", "Gone99"],
        "格式不对的码不去查"
    );
}

struct Fixture {
    service: PublicRoomDirectoryService,
    store: Arc<MemoryDirectory>,
}

impl Fixture {
    fn new(rooms: usize) -> Self {
        let store = Arc::new(MemoryDirectory {
            rooms: (0..rooms).map(room).collect(),
            ..MemoryDirectory::default()
        });
        let service = PublicRoomDirectoryService::new(PublicRoomDirectoryDependencies {
            directory: store.clone(),
            presence: Arc::new(EveryoneOnline),
            clock: Arc::new(TestClock),
        });
        Self { service, store }
    }
}

fn browse(viewer: Option<AuthenticatedPrincipal>) -> BrowsePublicRooms {
    BrowsePublicRooms {
        viewer,
        text: String::new(),
        join_mode: None,
        recently_active: false,
        unencrypted_only: false,
        sort: PublicRoomSort::Active,
        offset: 0,
    }
}

fn room(index: usize) -> PublicRoomRecord {
    PublicRoomRecord {
        catalog_id: room_id(index),
        name: format!("房间 {index}"),
        description: String::new(),
        owner_display_name: "aming@example.test".to_owned(),
        member_count: 3,
        agent_count: 1,
        join_mode: RoomJoinMode::Open,
        end_to_end_encrypted: false,
        share_code: RoomShareCode::parse(&format!("Room{index:02}")).unwrap(),
        created_at: time(NOW - 10_000),
        last_message_at: None,
    }
}

fn room_id(index: usize) -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(100 + index as u128))
}

fn viewer_id() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(7))
}

fn viewer(expires_at: i64) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: viewer_id(),
        matrix_user_id: "@viewer:matrix.test".to_owned(),
        display_name: "Viewer".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: time(expires_at),
        recently_authenticated: false,
    }
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

/// 不管翻到哪儿都给前 `limit` 间，看服务怎么算下一页；第一间房登录的人在等批准。
#[derive(Default)]
struct MemoryDirectory {
    rooms: Vec<PublicRoomRecord>,
    searches: Mutex<Vec<PublicRoomSearch>>,
    codes: Mutex<Vec<String>>,
    viewer_lookups: Mutex<Vec<(PrincipalId, Vec<RoomCatalogId>)>>,
}

impl PublicRoomDirectoryStore for MemoryDirectory {
    fn search_public_rooms<'a>(
        &'a self,
        search: &'a PublicRoomSearch,
    ) -> PortFuture<'a, RepositoryResult<Vec<PublicRoomRecord>>> {
        self.searches.lock().unwrap().push(search.clone());
        let rooms = self
            .rooms
            .iter()
            .take(search.limit as usize)
            .cloned()
            .collect();
        Box::pin(async move { Ok(rooms) })
    }

    fn public_room_by_share_code<'a>(
        &'a self,
        share_code: &'a RoomShareCode,
    ) -> PortFuture<'a, RepositoryResult<Option<PublicRoomDetailRecord>>> {
        self.codes
            .lock()
            .unwrap()
            .push(share_code.as_str().to_owned());
        let found = self
            .rooms
            .iter()
            .find(|room| room.share_code == *share_code)
            .map(|room| PublicRoomDetailRecord {
                room: room.clone(),
                admin_display_names: vec!["xiaolin@example.test".to_owned(), "老王".to_owned()],
            });
        Box::pin(async move { Ok(found) })
    }

    fn public_room_viewer_states<'a>(
        &'a self,
        viewer: PrincipalId,
        rooms: &'a [RoomCatalogId],
    ) -> PortFuture<'a, RepositoryResult<Vec<(RoomCatalogId, PublicRoomViewerState)>>> {
        self.viewer_lookups
            .lock()
            .unwrap()
            .push((viewer, rooms.to_vec()));
        let states = rooms
            .iter()
            .filter(|room| **room == room_id(0))
            .map(|room| (*room, PublicRoomViewerState::Pending))
            .collect();
        Box::pin(async move { Ok(states) })
    }

    fn public_room_presence_candidates(
        &self,
        _catalog_id: RoomCatalogId,
        _limit: u32,
    ) -> PortFuture<'_, RepositoryResult<Vec<MatrixUserId>>> {
        Box::pin(async { Ok(vec![MatrixUserId::new("@aming:matrix.test").unwrap()]) })
    }
}

struct EveryoneOnline;

impl PublicRoomPresenceMatrix for EveryoneOnline {
    fn presence<'a>(
        &'a self,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<MatrixPresenceState>> {
        Box::pin(async { Ok(MatrixPresenceState::Online) })
    }
}
