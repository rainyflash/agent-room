//! 公开房间的设置在 Matrix 上的两样副作用（`specs/public-rooms/design.md`），都以应用服务写房间
//! 状态：房间第一次公开的时间（`room.listing.v1`），和改回私密时打开加密（`m.room.encryption`）。
//! 房间的 `state_default` 是管理员那一级，只有应用服务够得着，所以读的一边看到这两条就知道是
//! 控制面写的。

use agent_room_application::ports::{
    MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId, PortFuture,
    PrivateRoomListingMatrix,
};
use agent_room_domain::time::UtcMillis;
use chrono::{DateTime, SecondsFormat};
use serde_json::{Value, json};

use super::{
    MatrixApplicationServiceProvisioner, map_transport_error,
    rooms::{endpoint_with_segments, expect_empty_success},
};

const ROOM_LISTING_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.room.listing.v1";

impl PrivateRoomListingMatrix for MatrixApplicationServiceProvisioner {
    fn mark_listed<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        first_listed_at: UtcMillis,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            let operation = MatrixOperation::SendStateEvent;
            let first_listed_at = DateTime::from_timestamp_millis(first_listed_at.value())
                .ok_or_else(|| {
                    MatrixFailure::new(operation, MatrixFailureKind::InvalidConfiguration)
                })?
                .to_rfc3339_opts(SecondsFormat::Millis, true);
            put_room_state(
                self,
                room_id,
                ROOM_LISTING_EVENT_TYPE,
                &json!({
                    "schemaVersion": "1.0",
                    "eventType": ROOM_LISTING_EVENT_TYPE,
                    "firstListedAt": first_listed_at,
                }),
            )
            .await
        })
    }

    fn enable_encryption<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        // 和新建私人房间时的初始状态一样。
        Box::pin(put_room_state(
            self,
            room_id,
            "m.room.encryption",
            &ENCRYPTION_CONTENT,
        ))
    }
}

static ENCRYPTION_CONTENT: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    json!({
        "algorithm": "m.megolm.v1.aes-sha2",
        "rotation_period_ms": 604_800_000_u64,
        "rotation_period_msgs": 100_u64,
    })
});

/// 以应用服务写一条 `state_key` 为空的房间状态。和当前状态一模一样时 Synapse 直接回旧的事件，
/// 所以重写是幂等的。
pub(super) async fn put_room_state(
    provisioner: &MatrixApplicationServiceProvisioner,
    room_id: &MatrixRoomId,
    event_type: &str,
    content: &Value,
) -> MatrixResult<()> {
    let operation = MatrixOperation::SendStateEvent;
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
        .put(endpoint)
        .bearer_auth(provisioner.access_token.expose())
        .json(content)
        .send()
        .await
        .map_err(|error| map_transport_error(operation, &error))?;
    expect_empty_success(response, operation).await
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use agent_room_application::ports::{
        MatrixFailureKind, MatrixRoomId, PrivateRoomListingMatrix, SecretValue,
    };
    use agent_room_domain::time::UtcMillis;
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
    async fn 第一次公开写下时间_改回私密写上加密_都用应用服务的身份() {
        let (url, writes) = start(StatusCode::OK).await;
        let provisioner = provisioner(&url);
        let room = MatrixRoomId::new("!club:matrix.agent-room.localhost").unwrap();

        provisioner
            .mark_listed(&room, UtcMillis::new(1_793_606_400_123).unwrap())
            .await
            .expect("写下公开的时间");
        provisioner
            .enable_encryption(&room)
            .await
            .expect("打开加密");

        let writes = writes.lock().await.clone();
        assert_eq!(writes.len(), 2);
        let (room_id, event_type, content, token) = &writes[0];
        assert_eq!(room_id, room.as_str());
        assert_eq!(event_type, "io.github.rainyflash.agentroom.room.listing.v1");
        assert_eq!(
            content,
            &json!({
                "schemaVersion": "1.0",
                "eventType": "io.github.rainyflash.agentroom.room.listing.v1",
                "firstListedAt": "2026-11-02T08:00:00.123Z",
            })
        );
        assert_eq!(token.as_deref(), Some("Bearer application-service-secret"));
        assert_eq!(writes[1].1, "m.room.encryption");
        assert_eq!(writes[1].2["algorithm"], "m.megolm.v1.aes-sha2");
    }

    #[tokio::test]
    async fn 聊天服务器拒绝时报错() {
        let (url, _) = start(StatusCode::SERVICE_UNAVAILABLE).await;
        let room = MatrixRoomId::new("!club:matrix.agent-room.localhost").unwrap();
        let failure = provisioner(&url)
            .enable_encryption(&room)
            .await
            .expect_err("没写进去");
        assert_ne!(failure.kind(), MatrixFailureKind::InvalidConfiguration);
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
