use std::{collections::HashMap, sync::Arc, time::Duration};

use agent_room_application::ports::{
    MatrixRoomId, MatrixUserId, PinTarget, PrivateRoomPinsGateway, RoomPinsChange,
    RoomPinsWriteFailure, SecretValue,
};
use agent_room_domain::{room_pins::RoomPinEventId, time::UtcMillis};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::Mutex};

use crate::{MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner};

const ROOM: &str = "!club:matrix.agent-room.localhost";
const OWNER: &str = "@owner:matrix.agent-room.localhost";
const AT: i64 = 1_793_606_400_123;

/// 假的聊天服务器：记着置顶那条状态、写了几次，按事件 ID 给事件。
#[derive(Default)]
struct Homeserver {
    pinned: Mutex<Option<Value>>,
    writes: Mutex<Vec<Value>>,
    events: HashMap<String, Value>,
    /// 读状态以后先停一下，让两次改置顶交错起来。
    read_delay: Option<Duration>,
}

#[tokio::test]
async fn 置顶写进标准的列表_另记谁什么时候_新的在最前面() {
    let server = Arc::new(Homeserver::default());
    let provisioner = start(server.clone()).await;

    let pins = provisioner
        .change_pins(&room(), &change("$first", true, AT))
        .await
        .expect("置顶第一句");
    assert_eq!(pins.entries().len(), 1);
    provisioner
        .change_pins(&room(), &change("$second", true, AT + 1_000))
        .await
        .expect("再置顶一句");

    let state = server.pinned.lock().await.clone().unwrap();
    assert_eq!(state["pinned"], json!(["$second", "$first"]));
    assert_eq!(
        state["io.github.rainyflash.agentroom.pins"],
        json!({
            "schemaVersion": "1.0",
            "entries": [
                {"eventId": "$second", "pinnedBy": OWNER, "pinnedAt": "2026-11-02T08:00:01.123Z"},
                {"eventId": "$first", "pinnedBy": OWNER, "pinnedAt": "2026-11-02T08:00:00.123Z"},
            ],
            "updatedBy": OWNER,
            "updatedAt": "2026-11-02T08:00:01.123Z",
            "change": {"kind": "pinned", "eventId": "$second"},
        })
    );
}

#[tokio::test]
async fn 取消置顶_没变的不写_满了不收() {
    let server = Arc::new(Homeserver::default());
    let provisioner = start(server.clone()).await;
    for n in 1..=5 {
        provisioner
            .change_pins(&room(), &change(&format!("${n}"), true, AT + n))
            .await
            .expect("五条以内");
    }
    assert_eq!(
        provisioner
            .change_pins(&room(), &change("$6", true, AT + 6))
            .await
            .unwrap_err(),
        RoomPinsWriteFailure::Full
    );
    let again = provisioner
        .change_pins(&room(), &change("$3", true, AT + 7))
        .await
        .expect("已经置顶着的照旧");
    assert_eq!(again.entries().len(), 5);
    assert_eq!(server.writes.lock().await.len(), 5, "满了和没变的都不写");

    let pins = provisioner
        .change_pins(&room(), &change("$3", false, AT + 8))
        .await
        .expect("取消一条");
    let order: Vec<_> = pins.entries().iter().map(|p| p.event_id.as_str()).collect();
    assert_eq!(order, ["$5", "$4", "$2", "$1"]);
    let state = server.pinned.lock().await.clone().unwrap();
    assert_eq!(state["pinned"], json!(["$5", "$4", "$2", "$1"]));
    assert_eq!(
        state["io.github.rainyflash.agentroom.pins"]["change"],
        json!({"kind": "unpinned", "eventId": "$3"})
    );
    provisioner
        .change_pins(&room(), &change("$3", false, AT + 9))
        .await
        .expect("本来就没置顶的");
    assert_eq!(server.writes.lock().await.len(), 6);
}

#[tokio::test]
async fn 两个人同时置顶_两条都在() {
    let server = Arc::new(Homeserver {
        read_delay: Some(Duration::from_millis(50)),
        ..Homeserver::default()
    });
    let provisioner = start(server.clone()).await;
    let room = room();
    let (pin_a, pin_b) = (change("$a", true, AT), change("$b", true, AT));
    let (first, second) = tokio::join!(
        provisioner.change_pins(&room, &pin_a),
        provisioner.change_pins(&room, &pin_b),
    );
    first.expect("第一个");
    second.expect("第二个");
    let state = server.pinned.lock().await.clone().unwrap();
    let mut pinned: Vec<_> = state["pinned"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_owned())
        .collect();
    pinned.sort();
    assert_eq!(pinned, ["$a", "$b"]);
}

#[tokio::test]
async fn 标准列表里有我们没记的_不写回() {
    let server = Arc::new(Homeserver {
        pinned: Mutex::new(Some(json!({ "pinned": ["$other"] }))),
        ..Homeserver::default()
    });
    let provisioner = start(server.clone()).await;
    assert!(matches!(
        provisioner
            .change_pins(&room(), &change("$mine", true, AT))
            .await
            .unwrap_err(),
        RoomPinsWriteFailure::Matrix(_)
    ));
    assert!(server.writes.lock().await.is_empty());
}

#[tokio::test]
async fn 只有这间房里没删的消息能置顶() {
    let events = HashMap::from([
        (
            "$encrypted".to_owned(),
            json!({"type": "m.room.encrypted", "content": {}}),
        ),
        (
            "$preview".to_owned(),
            json!({"type": "io.github.rainyflash.agentroom.message.preview.v2", "content": {}}),
        ),
        (
            "$plain".to_owned(),
            json!({"type": "m.room.message", "content": {"body": "hi"}}),
        ),
        (
            "$state".to_owned(),
            json!({"type": "m.room.encrypted", "state_key": "", "content": {}}),
        ),
        (
            "$revision".to_owned(),
            json!({"type": "io.github.rainyflash.agentroom.message.revision.v2", "content": {}}),
        ),
        (
            "$deleted".to_owned(),
            json!({
                "type": "m.room.encrypted",
                "content": {},
                "unsigned": {"redacted_because": {"type": "m.room.redaction"}},
            }),
        ),
    ]);
    let provisioner = start(Arc::new(Homeserver {
        events,
        ..Homeserver::default()
    }))
    .await;
    for (id, expected) in [
        ("$encrypted", PinTarget::Message),
        ("$preview", PinTarget::Message),
        ("$plain", PinTarget::Message),
        ("$state", PinTarget::NotMessage),
        ("$revision", PinTarget::NotMessage),
        ("$deleted", PinTarget::NotMessage),
        ("$elsewhere", PinTarget::NotMessage),
    ] {
        let target = provisioner
            .pin_target(&room(), &RoomPinEventId::parse(id).unwrap())
            .await
            .expect("读得到");
        assert_eq!(target, expected, "{id}");
    }
}

fn room() -> MatrixRoomId {
    MatrixRoomId::new(ROOM).unwrap()
}

fn change(event_id: &str, pinned: bool, at: i64) -> RoomPinsChange {
    RoomPinsChange {
        event_id: RoomPinEventId::parse(event_id).unwrap(),
        pinned,
        actor: MatrixUserId::new(OWNER.to_owned()).unwrap(),
        at: UtcMillis::new(at).unwrap(),
    }
}

async fn start(server: Arc<Homeserver>) -> MatrixApplicationServiceProvisioner {
    let app = Router::new()
        .route(
            "/_matrix/client/v3/rooms/{room}/state/m.room.pinned_events",
            get(read_state),
        )
        .route(
            "/_matrix/client/v3/rooms/{room}/state/m.room.pinned_events/",
            axum::routing::put(write_state),
        )
        .route(
            "/_matrix/client/v3/rooms/{room}/event/{event}",
            get(read_event),
        )
        .with_state(server);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("测试端口可用");
    let address = listener.local_addr().expect("测试地址有效");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("测试服务可运行");
    });
    let configuration = MatrixApplicationServiceConfiguration::new(
        format!("http://{address}"),
        "matrix.agent-room.localhost",
        SecretValue::new("application-service-secret").expect("密钥有效"),
        Duration::from_secs(2),
    )
    .expect("配置有效");
    MatrixApplicationServiceProvisioner::new(configuration).expect("适配器有效")
}

async fn read_state(
    State(server): State<Arc<Homeserver>>,
    Path(room): Path<String>,
) -> (StatusCode, Json<Value>) {
    assert_eq!(room, ROOM);
    let state = server.pinned.lock().await.clone();
    if let Some(delay) = server.read_delay {
        tokio::time::sleep(delay).await;
    }
    match state {
        Some(content) => (StatusCode::OK, Json(content)),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"errcode": "M_NOT_FOUND", "error": "no state"})),
        ),
    }
}

async fn write_state(
    State(server): State<Arc<Homeserver>>,
    Path(room): Path<String>,
    Json(content): Json<Value>,
) -> (StatusCode, Json<Value>) {
    assert_eq!(room, ROOM);
    *server.pinned.lock().await = Some(content.clone());
    server.writes.lock().await.push(content);
    (StatusCode::OK, Json(json!({"event_id": "$state"})))
}

async fn read_event(
    State(server): State<Arc<Homeserver>>,
    Path((room, event)): Path<(String, String)>,
) -> (StatusCode, Json<Value>) {
    assert_eq!(room, ROOM);
    match server.events.get(&event) {
        Some(found) => (StatusCode::OK, Json(found.clone())),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"errcode": "M_NOT_FOUND", "error": "no event"})),
        ),
    }
}
