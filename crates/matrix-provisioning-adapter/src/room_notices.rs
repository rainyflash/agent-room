//! 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：以应用服务写房间状态
//! `room.notice.v1`，每间房一条，新写的盖掉旧的。房间的 `state_default` 是管理员那一级，只有
//! 应用服务够得着，所以读的一边只认应用服务写的那条；改的人记在 `updatedBy` 里。
//!
//! 子房间里父房间须知的副本（`room.parent_notice.v1`，`specs/redesign/brief-and-pins.md` 的 4N-3b）也在
//! 这里：读父房间的须知、读写子房间里的副本，都以应用服务。副本和须知一样多一个 `parentRoomId`。

use agent_room_application::ports::{
    MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId, MatrixUserId,
    ParentNoticeCopy, ParentNoticeGateway, PortFuture, PrivateRoomNoticeGateway, RoomNoticeWrite,
};
use agent_room_domain::{room_notices::RoomNoticeText, time::UtcMillis};
use chrono::{DateTime, SecondsFormat};
use serde_json::{Value, json};

use super::{
    MatrixApplicationServiceProvisioner, decode_json, decode_matrix_error, map_matrix_error,
    map_transport_error, read_limited_body, room_listing::put_room_state,
    rooms::endpoint_with_segments,
};

const ROOM_NOTICE_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.room.notice.v1";
const PARENT_NOTICE_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.room.parent_notice.v1";

impl PrivateRoomNoticeGateway for MatrixApplicationServiceProvisioner {
    fn set_notice<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        notice: &'a RoomNoticeWrite,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            let content = notice_content(ROOM_NOTICE_EVENT_TYPE, notice)?;
            put_room_state(self, room_id, ROOM_NOTICE_EVENT_TYPE, &content).await
        })
    }
}

impl ParentNoticeGateway for MatrixApplicationServiceProvisioner {
    fn room_notice<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<Option<RoomNoticeWrite>>> {
        Box::pin(async move {
            let content = read_room_state(self, room_id, ROOM_NOTICE_EVENT_TYPE).await?;
            Ok(content.and_then(|content| notice_from(&content, ROOM_NOTICE_EVENT_TYPE)))
        })
    }

    fn parent_notice<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<Option<ParentNoticeCopy>>> {
        Box::pin(async move {
            let content = read_room_state(self, room_id, PARENT_NOTICE_EVENT_TYPE).await?;
            Ok(content.and_then(|content| copy_from(&content)))
        })
    }

    fn set_parent_notice<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        copy: &'a ParentNoticeCopy,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            let mut content = notice_content(PARENT_NOTICE_EVENT_TYPE, &copy.notice)?;
            content["parentRoomId"] = Value::from(copy.parent_room_id.as_str());
            put_room_state(self, room_id, PARENT_NOTICE_EVENT_TYPE, &content).await
        })
    }
}

/// 须知那几样写成房间状态的内容。时间写到毫秒、带 `Z`，读回来再写出去一字不差。
fn notice_content(event_type: &str, notice: &RoomNoticeWrite) -> MatrixResult<Value> {
    let updated_at = DateTime::from_timestamp_millis(notice.updated_at.value())
        .ok_or_else(|| {
            MatrixFailure::new(
                MatrixOperation::SendStateEvent,
                MatrixFailureKind::InvalidConfiguration,
            )
        })?
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    Ok(json!({
        "schemaVersion": "1.0",
        "eventType": event_type,
        "text": notice.text.as_str(),
        "updatedBy": notice.updated_by.as_str(),
        "updatedAt": updated_at,
    }))
}

/// 读一条须知的内容；读不懂的（类型不对、缺字段、太长、带回车、时间不对）不算一版。
fn notice_from(content: &Value, event_type: &str) -> Option<RoomNoticeWrite> {
    if content.get("eventType").and_then(Value::as_str) != Some(event_type) {
        return None;
    }
    let text = RoomNoticeText::parse(content.get("text")?.as_str()?).ok()?;
    let updated_by = MatrixUserId::new(content.get("updatedBy")?.as_str()?.to_owned()).ok()?;
    let updated_at = DateTime::parse_from_rfc3339(content.get("updatedAt")?.as_str()?)
        .ok()?
        .timestamp_millis();
    Some(RoomNoticeWrite {
        text,
        updated_by,
        updated_at: UtcMillis::new(updated_at).ok()?,
    })
}

fn copy_from(content: &Value) -> Option<ParentNoticeCopy> {
    let notice = notice_from(content, PARENT_NOTICE_EVENT_TYPE)?;
    let parent_room_id =
        MatrixRoomId::new(content.get("parentRoomId")?.as_str()?.to_owned()).ok()?;
    Some(ParentNoticeCopy {
        notice,
        parent_room_id,
    })
}

/// 以应用服务读一条 `state_key` 为空的房间状态；房间里还没有这一条时为空。
pub(crate) async fn read_room_state(
    provisioner: &MatrixApplicationServiceProvisioner,
    room_id: &MatrixRoomId,
    event_type: &str,
) -> MatrixResult<Option<Value>> {
    let operation = MatrixOperation::ReadRoomState;
    let endpoint = endpoint_with_segments(
        &provisioner.homeserver_url,
        &[
            "_matrix",
            "client",
            "v3",
            "rooms",
            room_id.as_str(),
            "state",
            event_type,
            "",
        ],
        operation,
    )?;
    let response = provisioner
        .client
        .get(endpoint)
        .bearer_auth(provisioner.access_token.expose())
        .send()
        .await
        .map_err(|error| map_transport_error(operation, &error))?;
    let status = response.status();
    let body = read_limited_body(response, operation).await?;
    if status.is_success() {
        return decode_json(&body, operation).map(Some);
    }
    let error = decode_matrix_error(&body, operation)?;
    let failure = map_matrix_error(operation, status, &error);
    if failure.kind() == MatrixFailureKind::NotFound {
        return Ok(None);
    }
    Err(failure)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use agent_room_application::ports::{
        MatrixRoomId, MatrixUserId, PrivateRoomNoticeGateway, RoomNoticeWrite, SecretValue,
    };
    use agent_room_domain::{room_notices::RoomNoticeText, time::UtcMillis};
    use axum::{
        Json, Router,
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
        routing::put,
    };
    use serde_json::{Value, json};
    use tokio::{net::TcpListener, sync::Mutex};

    use crate::{MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner};

    type Writes = Arc<Mutex<Vec<(String, String, Value, Option<String>)>>>;

    #[tokio::test]
    async fn 以应用服务写公告_带着改的人和时间() {
        let (url, writes) = start(StatusCode::OK).await;
        let room = MatrixRoomId::new("!club:matrix.agent-room.localhost").unwrap();
        provisioner(&url)
            .set_notice(&room, &notice("只在被点名时说话。\n回复要短。"))
            .await
            .expect("写进公告");

        let writes = writes.lock().await.clone();
        let (room_id, event_type, content, token) = &writes[0];
        assert_eq!(room_id, room.as_str());
        assert_eq!(event_type, "io.github.rainyflash.agentroom.room.notice.v1");
        assert_eq!(
            content,
            &json!({
                "schemaVersion": "1.0",
                "eventType": "io.github.rainyflash.agentroom.room.notice.v1",
                "text": "只在被点名时说话。\n回复要短。",
                "updatedBy": "@owner:matrix.agent-room.localhost",
                "updatedAt": "2026-11-02T08:00:00.123Z",
            })
        );
        assert_eq!(token.as_deref(), Some("Bearer application-service-secret"));
    }

    #[tokio::test]
    async fn 聊天服务器拒绝时报错() {
        let (url, writes) = start(StatusCode::SERVICE_UNAVAILABLE).await;
        let room = MatrixRoomId::new("!club:matrix.agent-room.localhost").unwrap();
        provisioner(&url)
            .set_notice(&room, &notice(""))
            .await
            .expect_err("没写进去");
        assert!(writes.lock().await.is_empty());
    }

    fn notice(text: &str) -> RoomNoticeWrite {
        RoomNoticeWrite {
            text: RoomNoticeText::parse(text).unwrap(),
            updated_by: MatrixUserId::new("@owner:matrix.agent-room.localhost".to_owned()).unwrap(),
            updated_at: UtcMillis::new(1_793_606_400_123).unwrap(),
        }
    }

    async fn start(status: StatusCode) -> (String, Writes) {
        let writes = Writes::default();
        let app = Router::new()
            .route(
                "/_matrix/client/v3/rooms/{room}/state/{event_type}/",
                put(record),
            )
            .with_state((writes.clone(), status));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("测试端口可用");
        let address = listener.local_addr().expect("测试地址有效");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("测试服务可运行");
        });
        (format!("http://{address}"), writes)
    }

    async fn record(
        State((writes, status)): State<(Writes, StatusCode)>,
        Path((room, event_type)): Path<(String, String)>,
        headers: HeaderMap,
        Json(content): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        if !status.is_success() {
            return (
                status,
                Json(json!({ "errcode": "M_UNKNOWN", "error": "unavailable" })),
            );
        }
        let token = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        writes.lock().await.push((room, event_type, content, token));
        (status, Json(json!({ "event_id": "$state" })))
    }

    fn provisioner(url: &str) -> MatrixApplicationServiceProvisioner {
        let configuration = MatrixApplicationServiceConfiguration::new(
            url,
            "matrix.agent-room.localhost",
            SecretValue::new("application-service-secret").expect("密钥有效"),
            Duration::from_secs(2),
        )
        .expect("配置有效");
        MatrixApplicationServiceProvisioner::new(configuration).expect("适配器有效")
    }
}

#[cfg(test)]
mod parent_notice_tests {
    use std::{collections::HashMap, sync::Arc, time::Duration};

    use agent_room_application::ports::{
        MatrixFailureKind, MatrixRoomId, MatrixUserId, ParentNoticeCopy, ParentNoticeGateway,
        RoomNoticeWrite, SecretValue,
    };
    use agent_room_domain::{room_notices::RoomNoticeText, time::UtcMillis};
    use axum::{
        Json, Router,
        extract::{Path, State},
        http::{HeaderMap, StatusCode},
        routing::get,
    };
    use serde_json::{Value, json};
    use tokio::{net::TcpListener, sync::Mutex};

    use crate::{MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner};

    const NOTICE: &str = "io.github.rainyflash.agentroom.room.notice.v1";
    const COPY: &str = "io.github.rainyflash.agentroom.room.parent_notice.v1";
    const PARENT: &str = "!parent:matrix.agent-room.localhost";
    const SUB_ROOM: &str = "!sub:matrix.agent-room.localhost";
    const OWNER: &str = "@owner:matrix.agent-room.localhost";
    const AT: &str = "2026-11-02T08:00:00.123Z";

    /// 假的聊天服务器：按房间和类型存房间状态，没有的回 404；`unavailable` 时什么都回 503。读写都记下
    /// 带的令牌。
    #[derive(Clone, Default)]
    struct Homeserver {
        state: Arc<Mutex<HashMap<(String, String), Value>>>,
        tokens: Arc<Mutex<Vec<Option<String>>>>,
        unavailable: bool,
    }

    impl Homeserver {
        async fn put(&self, room: &str, event_type: &str, content: Value) {
            self.state
                .lock()
                .await
                .insert((room.to_owned(), event_type.to_owned()), content);
        }

        async fn content(&self, room: &str, event_type: &str) -> Option<Value> {
            self.state
                .lock()
                .await
                .get(&(room.to_owned(), event_type.to_owned()))
                .cloned()
        }
    }

    #[tokio::test]
    async fn 读父房间的须知_没写过的回空_读不懂的当没有() {
        let homeserver = Homeserver::default();
        let url = start(homeserver.clone()).await;
        let matrix = provisioner(&url);
        let parent = room(PARENT);

        assert_eq!(matrix.room_notice(&parent).await.unwrap(), None);

        let written = json!({
            "schemaVersion": "1.0",
            "eventType": NOTICE,
            "text": "先说要做什么。\n再动手。",
            "updatedBy": OWNER,
            "updatedAt": AT,
        });
        homeserver.put(PARENT, NOTICE, written).await;
        assert_eq!(
            matrix.room_notice(&parent).await.unwrap(),
            Some(notice("先说要做什么。\n再动手。"))
        );
        let last_token = homeserver.tokens.lock().await.last().cloned().flatten();
        assert_eq!(
            last_token.as_deref(),
            Some("Bearer application-service-secret")
        );

        for broken in [
            json!({ "eventType": NOTICE, "text": "缺了改的人", "updatedAt": AT }),
            json!({ "eventType": NOTICE, "text": "时间不对", "updatedBy": OWNER, "updatedAt": "yesterday" }),
            json!({ "eventType": NOTICE, "text": "回车\r不收", "updatedBy": OWNER, "updatedAt": AT }),
            json!({ "eventType": COPY, "text": "类型不对", "updatedBy": OWNER, "updatedAt": AT }),
        ] {
            homeserver.put(PARENT, NOTICE, broken.clone()).await;
            assert_eq!(matrix.room_notice(&parent).await.unwrap(), None, "{broken}");
        }
    }

    #[tokio::test]
    async fn 写子房间里的副本_带着父房间的房间号_读回来一样() {
        let homeserver = Homeserver::default();
        let url = start(homeserver.clone()).await;
        let matrix = provisioner(&url);
        let sub_room = room(SUB_ROOM);
        assert_eq!(matrix.parent_notice(&sub_room).await.unwrap(), None);

        let copy = ParentNoticeCopy {
            notice: notice("先说要做什么。"),
            parent_room_id: room(PARENT),
        };
        matrix
            .set_parent_notice(&sub_room, &copy)
            .await
            .expect("写得进");

        assert_eq!(
            homeserver.content(SUB_ROOM, COPY).await,
            Some(json!({
                "schemaVersion": "1.0",
                "eventType": COPY,
                "text": "先说要做什么。",
                "updatedBy": OWNER,
                "updatedAt": AT,
                "parentRoomId": PARENT,
            }))
        );
        assert_eq!(
            homeserver.content(SUB_ROOM, NOTICE).await,
            None,
            "没碰子房间自己的须知"
        );
        assert_eq!(matrix.parent_notice(&sub_room).await.unwrap(), Some(copy));

        // 少了父房间的房间号、类型写成须知的，都不算副本。
        let mut without_parent = homeserver.content(SUB_ROOM, COPY).await.unwrap();
        without_parent
            .as_object_mut()
            .unwrap()
            .remove("parentRoomId");
        homeserver.put(SUB_ROOM, COPY, without_parent).await;
        assert_eq!(matrix.parent_notice(&sub_room).await.unwrap(), None);
        let wrong_type = json!({
            "eventType": NOTICE,
            "text": "先说要做什么。",
            "updatedBy": OWNER,
            "updatedAt": AT,
            "parentRoomId": PARENT,
        });
        homeserver.put(SUB_ROOM, COPY, wrong_type).await;
        assert_eq!(matrix.parent_notice(&sub_room).await.unwrap(), None);
    }

    #[tokio::test]
    async fn 聊天服务器用不了时报错_不当成没有() {
        let homeserver = Homeserver {
            unavailable: true,
            ..Homeserver::default()
        };
        let url = start(homeserver.clone()).await;
        let matrix = provisioner(&url);

        let read = matrix.room_notice(&room(PARENT)).await.unwrap_err();
        assert_eq!(read.kind(), MatrixFailureKind::DependencyUnavailable);
        let copy = matrix.parent_notice(&room(SUB_ROOM)).await.unwrap_err();
        assert_eq!(copy.kind(), MatrixFailureKind::DependencyUnavailable);
        let removal = ParentNoticeCopy {
            notice: notice(""),
            parent_room_id: room(PARENT),
        };
        matrix
            .set_parent_notice(&room(SUB_ROOM), &removal)
            .await
            .expect_err("没写进去");
        assert!(homeserver.state.lock().await.is_empty());
    }

    fn room(id: &str) -> MatrixRoomId {
        MatrixRoomId::new(id.to_owned()).unwrap()
    }

    /// `AT` 那一刻、`OWNER` 改的一版。
    fn notice(text: &str) -> RoomNoticeWrite {
        RoomNoticeWrite {
            text: RoomNoticeText::parse(text).unwrap(),
            updated_by: MatrixUserId::new(OWNER.to_owned()).unwrap(),
            updated_at: UtcMillis::new(1_793_606_400_123).unwrap(),
        }
    }

    async fn start(homeserver: Homeserver) -> String {
        let app = Router::new()
            .route(
                "/_matrix/client/v3/rooms/{room}/state/{event_type}/",
                get(read).put(write),
            )
            .with_state(homeserver);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("测试端口可用");
        let address = listener.local_addr().expect("测试地址有效");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("测试服务可运行");
        });
        format!("http://{address}")
    }

    fn token(headers: &HeaderMap) -> Option<String> {
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    fn unavailable() -> (StatusCode, Json<Value>) {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "errcode": "M_UNKNOWN", "error": "unavailable" })),
        )
    }

    async fn read(
        State(homeserver): State<Homeserver>,
        Path((room, event_type)): Path<(String, String)>,
        headers: HeaderMap,
    ) -> (StatusCode, Json<Value>) {
        homeserver.tokens.lock().await.push(token(&headers));
        if homeserver.unavailable {
            return unavailable();
        }
        match homeserver.content(&room, &event_type).await {
            Some(content) => (StatusCode::OK, Json(content)),
            None => (
                StatusCode::NOT_FOUND,
                Json(json!({ "errcode": "M_NOT_FOUND", "error": "Event not found." })),
            ),
        }
    }

    async fn write(
        State(homeserver): State<Homeserver>,
        Path((room, event_type)): Path<(String, String)>,
        headers: HeaderMap,
        Json(content): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        homeserver.tokens.lock().await.push(token(&headers));
        if homeserver.unavailable {
            return unavailable();
        }
        homeserver.put(&room, &event_type, content).await;
        (StatusCode::OK, Json(json!({ "event_id": "$state" })))
    }

    fn provisioner(url: &str) -> MatrixApplicationServiceProvisioner {
        let configuration = MatrixApplicationServiceConfiguration::new(
            url,
            "matrix.agent-room.localhost",
            SecretValue::new("application-service-secret").expect("密钥有效"),
            Duration::from_secs(2),
        )
        .expect("配置有效");
        MatrixApplicationServiceProvisioner::new(configuration).expect("适配器有效")
    }
}
