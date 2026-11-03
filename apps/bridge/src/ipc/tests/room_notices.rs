//! 房间的常驻公告（`ReadRoomNotices`），用真的本地消息库。

use agent_room_application::ports::MatrixRoomId;
use agent_room_bridge_core::messages::{RoomNotice, RoomStateChange};
use agent_room_bridge_ipc::{IpcMethod, IpcReadRoomNoticesRequest, IpcResponse, IpcRoomNotice};
use agent_room_bridge_storage_adapter::SqliteMessageTimelineRepository;
use agent_room_domain::time::UtcMillis;

use super::super::BridgeIpcRequestHandler as _;
use super::FoundationBridgeIpcRequestHandler;
use super::viewing::{ROOM, handler, store_with};

async fn record(store: &SqliteMessageTimelineRepository, text: Option<&str>, at_ms: i64) {
    store
        .record_room_state(
            &[RoomStateChange {
                room_id: MatrixRoomId::new(ROOM).expect("房间标识有效"),
                name: None,
                membership: None,
                notice: Some(RoomNotice {
                    text: text.map(str::to_owned),
                    updated_by: "@owner:matrix.test".to_owned(),
                    updated_at_ms: at_ms,
                }),
            }],
            UtcMillis::new(9_000).expect("时间有效"),
        )
        .await
        .expect("可以记");
}

async fn read(
    handler: &FoundationBridgeIpcRequestHandler,
    room_id: Option<&str>,
    changed_only: bool,
) -> Vec<IpcRoomNotice> {
    let method = IpcMethod::ReadRoomNotices(IpcReadRoomNoticesRequest {
        room_id: room_id.map(str::to_owned),
        changed_only,
    });
    match handler.dispatch(method).await.expect("可以读公告") {
        IpcResponse::RoomNotices { notices } => notices,
        other => panic!("读公告必须返回公告：{other:?}"),
    }
}

fn notice(text: Option<&str>, at_ms: i64) -> IpcRoomNotice {
    IpcRoomNotice {
        room_id: ROOM.to_owned(),
        text: text.map(str::to_owned),
        updated_by: "@owner:matrix.test".to_owned(),
        updated_at_unix_ms: at_ms,
    }
}

#[tokio::test]
async fn 进房间拿现在的_改了以后只拿一次_撤掉也说一次() {
    let (_directory, store) = store_with(&[]).await;
    let handler = handler(store.clone());
    assert_eq!(read(&handler, Some(ROOM), false).await, [], "还没有公告");

    record(&store, Some("只在被点名时说话。"), 1_000).await;
    // 进房间（给了房间）拿现在的；之后等消息时就不再给同一版。
    assert_eq!(
        read(&handler, Some(ROOM), false).await,
        [notice(Some("只在被点名时说话。"), 1_000)]
    );
    assert_eq!(read(&handler, None, true).await, []);

    record(&store, Some("回复要短。"), 2_000).await;
    assert_eq!(
        read(&handler, None, true).await,
        [notice(Some("回复要短。"), 2_000)]
    );
    assert_eq!(read(&handler, None, true).await, [], "改一次只拿一次");

    record(&store, None, 3_000).await;
    assert_eq!(read(&handler, None, true).await, [notice(None, 3_000)]);
    assert_eq!(read(&handler, Some(ROOM), false).await, [], "撤掉了就没有公告");
}
