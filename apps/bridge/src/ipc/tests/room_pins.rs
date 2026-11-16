//! 房间的置顶（`ReadRoomPins`），用真的本地消息库。

use agent_room_application::ports::{MatrixEventId, MatrixRoomId};
use agent_room_bridge_core::messages::{RoomPinEntry, RoomPinsState, RoomStateChange};
use agent_room_bridge_ipc::{
    IpcMethod, IpcPinUnreadable, IpcReadRoomPinsRequest, IpcResponse, IpcRoomPins,
};
use agent_room_bridge_storage_adapter::SqliteMessageTimelineRepository;
use agent_room_domain::time::UtcMillis;

use super::super::BridgeIpcRequestHandler as _;
use super::FoundationBridgeIpcRequestHandler;
use super::viewing::{ROOM, chat, event, handler, human, store_with};

async fn record(store: &SqliteMessageTimelineRepository, pinned: &[u8], at_ms: i64) {
    store
        .record_room_state(
            &[RoomStateChange {
                room_id: MatrixRoomId::new(ROOM).expect("房间标识有效"),
                name: None,
                membership: None,
                notice: None,
                parent_notice: None,
                pins: Some(RoomPinsState {
                    entries: pinned
                        .iter()
                        .map(|index| RoomPinEntry {
                            event_id: MatrixEventId::new(event(*index)).expect("事件标识有效"),
                            pinned_by: "@owner:matrix.test".to_owned(),
                            pinned_at_ms: at_ms,
                        })
                        .collect(),
                    updated_by: "@owner:matrix.test".to_owned(),
                    updated_at_ms: at_ms,
                }),
            }],
            UtcMillis::new(9_000).expect("时间有效"),
        )
        .await
        .expect("可以记");
}

/// 进房间：要现在的。
const ENTER: IpcReadRoomPinsRequest = IpcReadRoomPinsRequest {
    room_id: None,
    every_room: false,
    changed_only: false,
};
/// 交一批消息：Agent 在的所有房间、只要改了的。
const BATCH: IpcReadRoomPinsRequest = IpcReadRoomPinsRequest {
    room_id: None,
    every_room: true,
    changed_only: true,
};

async fn read(
    handler: &FoundationBridgeIpcRequestHandler,
    request: IpcReadRoomPinsRequest,
) -> Vec<IpcRoomPins> {
    match handler
        .dispatch(IpcMethod::ReadRoomPins(request))
        .await
        .expect("可以读置顶")
    {
        IpcResponse::RoomPins { rooms } => rooms,
        other => panic!("读置顶必须返回置顶：{other:?}"),
    }
}

fn pinned_events(rooms: &[IpcRoomPins]) -> Vec<Vec<String>> {
    rooms
        .iter()
        .map(|room| room.pinned.iter().map(|pin| pin.event_id.clone()).collect())
        .collect()
}

#[tokio::test]
async fn 进房间拿现在的整份_改了以后只拿一次_全取消了也说一次() {
    let ada = human("Ada", "@ada:matrix.test");
    let (_directory, store) =
        store_with(&[chat(ROOM, 1, ada, "周五发版，别合改工作流的。", &[])]).await;
    let handler = handler(store.clone());
    assert_eq!(read(&handler, ENTER).await, [], "还没有置顶");

    record(&store, &[1, 2], 1_000).await;
    let entered = read(&handler, ENTER).await;
    assert_eq!(pinned_events(&entered), [[event(1), event(2)]]);
    let [room] = entered.as_slice() else {
        panic!("一个房间：{entered:?}");
    };
    assert_eq!(room.room_id, ROOM);
    assert_eq!(room.updated_at_unix_ms, 1_000);
    // 消息库里有的那一句带上谁说的、开头；还没有的写明。
    let known = room.pinned[0].message.as_ref().expect("那一句在消息库里");
    assert_eq!(known.actor_name, "Ada");
    assert_eq!(known.excerpt, "周五发版，别合改工作流的。");
    assert_eq!(room.pinned[0].unreadable, None);
    assert_eq!(room.pinned[1].message, None);
    assert_eq!(room.pinned[1].unreadable, Some(IpcPinUnreadable::NotLoaded));
    assert_eq!(read(&handler, BATCH).await, [], "进房间时交过了");

    // 只往新的改：同一版、更早的一版都不算改。
    record(&store, &[1, 2], 1_000).await;
    record(&store, &[3], 500).await;
    assert_eq!(read(&handler, BATCH).await, []);

    record(&store, &[2], 2_000).await;
    assert_eq!(pinned_events(&read(&handler, BATCH).await), [[event(2)]]);
    assert_eq!(read(&handler, BATCH).await, [], "改一次只拿一次");

    record(&store, &[], 3_000).await;
    let cleared = read(&handler, BATCH).await;
    assert_eq!(
        pinned_events(&cleared),
        [Vec::<String>::new()],
        "全取消了也说"
    );
    assert_eq!(read(&handler, ENTER).await, [], "没有置顶时进房间不给");
}

#[tokio::test]
async fn 从没交过的全取消了不说() {
    let (_directory, store) = store_with(&[]).await;
    let handler = handler(store.clone());
    record(&store, &[], 1_000).await;
    assert_eq!(read(&handler, BATCH).await, []);
}
