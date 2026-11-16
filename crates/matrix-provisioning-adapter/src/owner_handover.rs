//! 房主删除了账户、房间交给了管理员（`specs/redesign/room-roles.md` 的“房主不在了（R6）”）：以
//! 应用服务写房间状态 `room.owner_handover.v1`，网页照它在对话里写一行。只有应用服务够得着这一级
//! 的状态，读的一边只认应用服务写的那条。
//!
//! 内容里不带时间：删除任务重来时写一样的内容，聊天服务器直接回旧的事件，不会多出一行。

use agent_room_application::ports::{
    MatrixResult, MatrixRoomId, OwnerHandoverWrite, PortFuture, PrivateRoomOwnerHandoverGateway,
};
use serde_json::json;

use super::{MatrixApplicationServiceProvisioner, room_listing::put_room_state};

const OWNER_HANDOVER_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.room.owner_handover.v1";

impl PrivateRoomOwnerHandoverGateway for MatrixApplicationServiceProvisioner {
    fn announce_owner_handover<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        handover: &'a OwnerHandoverWrite,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            put_room_state(
                self,
                room_id,
                OWNER_HANDOVER_EVENT_TYPE,
                &json!({
                    "schemaVersion": "1.0",
                    "eventType": OWNER_HANDOVER_EVENT_TYPE,
                    "owner": handover.owner.as_str(),
                    "formerOwner": handover.former_owner.as_str(),
                }),
            )
            .await
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use agent_room_application::ports::{
        MatrixRoomId, MatrixUserId, OwnerHandoverWrite, PrivateRoomOwnerHandoverGateway,
        SecretValue,
    };
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
    async fn 以应用服务写交给了谁_带着原房主_不带时间() {
        let (url, writes) = start(StatusCode::OK).await;
        let room = MatrixRoomId::new("!club:matrix.agent-room.localhost").unwrap();
        provisioner(&url)
            .announce_owner_handover(&room, &handover())
            .await
            .expect("写进去");

        let writes = writes.lock().await.clone();
        let (room_id, event_type, content, token) = &writes[0];
        assert_eq!(room_id, room.as_str());
        assert_eq!(
            event_type,
            "io.github.rainyflash.agentroom.room.owner_handover.v1"
        );
        assert_eq!(
            content,
            &json!({
                "schemaVersion": "1.0",
                "eventType": "io.github.rainyflash.agentroom.room.owner_handover.v1",
                "owner": "@admin:matrix.agent-room.localhost",
                "formerOwner": "@owner:matrix.agent-room.localhost",
            })
        );
        assert_eq!(token.as_deref(), Some("Bearer application-service-secret"));
    }

    #[tokio::test]
    async fn 聊天服务器拒绝时报错() {
        let (url, writes) = start(StatusCode::SERVICE_UNAVAILABLE).await;
        let room = MatrixRoomId::new("!club:matrix.agent-room.localhost").unwrap();
        provisioner(&url)
            .announce_owner_handover(&room, &handover())
            .await
            .expect_err("没写进去");
        assert!(writes.lock().await.is_empty());
    }

    fn handover() -> OwnerHandoverWrite {
        OwnerHandoverWrite {
            owner: MatrixUserId::new("@admin:matrix.agent-room.localhost".to_owned()).unwrap(),
            former_owner: MatrixUserId::new("@owner:matrix.agent-room.localhost".to_owned())
                .unwrap(),
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
