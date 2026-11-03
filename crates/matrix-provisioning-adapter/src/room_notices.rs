//! 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：以应用服务写房间状态
//! `room.notice.v1`，每间房一条，新写的盖掉旧的。房间的 `state_default` 是管理员那一级，只有
//! 应用服务够得着，所以读的一边只认应用服务写的那条；改的人记在 `updatedBy` 里。

use agent_room_application::ports::{
    MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId, PortFuture,
    PrivateRoomNoticeGateway, RoomNoticeWrite,
};
use chrono::{DateTime, SecondsFormat};
use serde_json::json;

use super::{MatrixApplicationServiceProvisioner, room_listing::put_room_state};

const ROOM_NOTICE_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.room.notice.v1";

impl PrivateRoomNoticeGateway for MatrixApplicationServiceProvisioner {
    fn set_notice<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        notice: &'a RoomNoticeWrite,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            let updated_at = DateTime::from_timestamp_millis(notice.updated_at.value())
                .ok_or_else(|| {
                    MatrixFailure::new(
                        MatrixOperation::SendStateEvent,
                        MatrixFailureKind::InvalidConfiguration,
                    )
                })?
                .to_rfc3339_opts(SecondsFormat::Millis, true);
            put_room_state(
                self,
                room_id,
                ROOM_NOTICE_EVENT_TYPE,
                &json!({
                    "schemaVersion": "1.0",
                    "eventType": ROOM_NOTICE_EVENT_TYPE,
                    "text": notice.text.as_str(),
                    "updatedBy": notice.updated_by.as_str(),
                    "updatedAt": updated_at,
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
