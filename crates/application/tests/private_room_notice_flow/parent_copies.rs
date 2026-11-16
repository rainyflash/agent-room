//! 子房间里父房间须知的副本（`specs/redesign/brief-and-pins.md` 的 4N-3b）：父房间改了须知，下面每间子房间
//! （归档了的也算）当场写上一份，带着父房间的房间号；撤掉了跟着清空；改的是子房间自己的不排；没写成
//! 不算请求失败，后台到点补上；整个聊天服务器用不了时这一批剩下的不再试。

use agent_room_application::private_rooms::{ParentNoticeSyncRetry, ParentNoticeSyncUseCases};

use super::*;

const PARENT_ROOM: &str = "!club:matrix.test";
const ALPHA_ROOM: &str = "!alpha:matrix.test";
const BETA_ROOM: &str = "!beta:matrix.test";

fn alpha() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(11))
}

fn beta() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(12))
}

/// 一间子房间，房主和父房间一样，成员是房主和那位管理员。
fn sub_room(id: RoomCatalogId, archived: bool) -> PrivateRoom {
    let mut room = PrivateRoom::create(id, owner());
    room.invite(owner(), manager(), PrivateRoomPermissions::ALL)
        .unwrap();
    room.accept_invitation(manager()).unwrap();
    if archived {
        room.archive(owner()).unwrap();
    }
    room
}

/// 父房间，下面一间在用的 Alpha、一间归档了的 Beta。
fn family() -> Fixture {
    Fixture::with_rooms(vec![
        snapshot(room()),
        snapshot_in(sub_room(alpha(), false), ALPHA_ROOM).under(catalog_id()),
        snapshot_in(sub_room(beta(), true), BETA_ROOM).under(catalog_id()),
    ])
}

/// 照父房间那一版该写成的副本。
fn copy(text: &str, by: PrincipalId, at: i64) -> ParentNoticeCopy {
    ParentNoticeCopy {
        notice: RoomNoticeWrite {
            text: agent_room_domain::room_notices::RoomNoticeText::parse(text).unwrap(),
            updated_by: agent_room_application::ports::MatrixUserId::new(matrix_user(by)).unwrap(),
            updated_at: time(at),
        },
        parent_room_id: MatrixRoomId::new(PARENT_ROOM.to_owned()).unwrap(),
    }
}

#[tokio::test]
async fn 父房间改了须知_下面每间子房间当场写上副本_归档了的也写_撤掉了跟着清空() {
    let fixture = family();

    fixture
        .service
        .update_notice(update(owner(), "先说要做什么，再动手。"))
        .await
        .expect("房主能写");
    let written = copy("先说要做什么，再动手。", owner(), NOW);
    assert_eq!(
        fixture.homeserver.copy_of(ALPHA_ROOM),
        Some(written.clone())
    );
    assert_eq!(fixture.homeserver.copy_of(BETA_ROOM), Some(written));
    assert!(fixture.queue.is_empty(), "写完的都删了");
    // 父房间自己那条照旧只写一次。
    assert_eq!(fixture.writes.lock().unwrap().len(), 1);

    fixture.clock.advance(30_000);
    fixture
        .service
        .update_notice(update(manager(), ""))
        .await
        .expect("管理员能撤掉");
    let removed = copy("", manager(), NOW + 30_000);
    assert_eq!(
        fixture.homeserver.copy_of(ALPHA_ROOM),
        Some(removed.clone())
    );
    assert_eq!(fixture.homeserver.copy_of(BETA_ROOM), Some(removed));
    assert!(fixture.queue.is_empty());
}

#[tokio::test]
async fn 改的是子房间自己的须知_不排副本() {
    let fixture = family();
    fixture
        .service
        .update_notice(UpdateRoomNotice {
            catalog_id: alpha(),
            ..update(owner(), "这一间只管发版。")
        })
        .await
        .expect("子房间的房主能写");

    assert!(fixture.queue.queued_under.lock().unwrap().is_empty());
    assert_eq!(*fixture.homeserver.copy_calls.lock().unwrap(), 0);
    assert_eq!(fixture.homeserver.copy_of(ALPHA_ROOM), None);
}

#[tokio::test]
async fn 副本没写成不算请求失败_整个聊天服务器用不了时剩下的不再试_到点补上() {
    let fixture = family();
    fixture
        .homeserver
        .fail_copies_in(ALPHA_ROOM, MatrixFailureKind::DependencyUnavailable);

    fixture
        .service
        .update_notice(update(owner(), "先说要做什么。"))
        .await
        .expect("副本没写成也照样回答");
    // Alpha 读副本时就失败了；Beta 照同一个原因往后排，没去试。
    assert_eq!(*fixture.homeserver.copy_calls.lock().unwrap(), 2);
    for sub_room in [alpha(), beta()] {
        let row = fixture.queue.row(sub_room).expect("还排着");
        assert_eq!(row.failure_code, Some("matrix.unavailable"));
        assert_eq!(row.next_attempt_at, Some(NOW + 30_000));
    }

    // 聊天服务器好了，没到点的不领。
    fixture.homeserver.copy_failures.lock().unwrap().clear();
    let early = fixture.service.sync_parent_notices().await.unwrap();
    assert_eq!(early.done, 0);

    fixture.clock.advance(30_000);
    let round = fixture.service.sync_parent_notices().await.unwrap();
    assert_eq!(round.done, 2);
    assert!(round.retrying.is_empty());
    let written = copy("先说要做什么。", owner(), NOW);
    assert_eq!(
        fixture.homeserver.copy_of(ALPHA_ROOM),
        Some(written.clone())
    );
    assert_eq!(fixture.homeserver.copy_of(BETA_ROOM), Some(written));
    assert!(fixture.queue.is_empty());
}

#[tokio::test]
async fn 一间被拒不耽误别的_退避翻倍() {
    let fixture = family();
    fixture
        .homeserver
        .fail_copies_in(ALPHA_ROOM, MatrixFailureKind::Forbidden);
    fixture
        .service
        .update_notice(update(owner(), "先说要做什么。"))
        .await
        .unwrap();
    assert_eq!(
        fixture.homeserver.copy_of(BETA_ROOM),
        Some(copy("先说要做什么。", owner(), NOW))
    );
    assert_eq!(fixture.queue.row(beta()), None);

    fixture.clock.advance(30_000);
    let round = fixture.service.sync_parent_notices().await.unwrap();
    assert_eq!(
        round.retrying,
        [ParentNoticeSyncRetry {
            failure_code: "matrix.forbidden",
            attempt: 2,
        }]
    );
    let row = fixture.queue.row(alpha()).expect("还排着");
    assert_eq!(row.next_attempt_at, Some(NOW + 30_000 + 60_000));
}

#[tokio::test]
async fn 后台一轮_新建的子房间照父房间写_父房间没有须知的不写_子房间不在了的删掉() {
    let fixture = family();
    // 新建子房间时存储在建房的事务里排上；父房间这时还没有须知。
    fixture.queue.queue(alpha());
    let round = fixture.service.sync_parent_notices().await.unwrap();
    assert_eq!(round.done, 1);
    assert_eq!(
        fixture.homeserver.copy_of(ALPHA_ROOM),
        None,
        "两边都没有，不写"
    );

    // 父房间有了须知以后新建的：照它写上。
    fixture.homeserver.notices.lock().unwrap().insert(
        PARENT_ROOM.to_owned(),
        copy("先说要做什么。", manager(), NOW - 5_000).notice,
    );
    fixture.queue.queue(beta());
    fixture.service.sync_parent_notices().await.unwrap();
    assert_eq!(
        fixture.homeserver.copy_of(BETA_ROOM),
        Some(copy("先说要做什么。", manager(), NOW - 5_000))
    );

    // 已经一样的再排一次：读一眼，不再写。
    fixture.queue.queue(beta());
    let calls = *fixture.homeserver.copy_calls.lock().unwrap();
    fixture.service.sync_parent_notices().await.unwrap();
    assert_eq!(*fixture.homeserver.copy_calls.lock().unwrap(), calls + 2);

    // 子房间不在了（库里找不到）：做完就删。
    let gone = RoomCatalogId::from_uuid(Uuid::from_u128(99));
    fixture.queue.queue(gone);
    let round = fixture.service.sync_parent_notices().await.unwrap();
    assert_eq!(round.done, 1);
    assert!(fixture.queue.is_empty());
}
