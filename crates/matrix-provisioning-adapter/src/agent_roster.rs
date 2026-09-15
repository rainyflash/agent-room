use crate::{
    MatrixApplicationServiceProvisioner,
    rooms::{endpoint_with_segments, expect_empty_success},
};
use agent_room_application::{
    agent_roster::{AGENT_ROSTER_POLICY_EVENT_TYPE, AgentRosterPolicyPublisher},
    ports::{MatrixOperation, MatrixResult, MatrixRoomId, PortFuture},
};
use agent_room_domain::agent_lifecycle::AgentRosterPolicy;

impl AgentRosterPolicyPublisher for MatrixApplicationServiceProvisioner {
    fn publish<'a>(
        &'a self,
        room: &'a MatrixRoomId,
        policy: AgentRosterPolicy,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            let operation = MatrixOperation::SendStateEvent;
            let endpoint = endpoint_with_segments(
                &self.homeserver_url,
                &[
                    "_matrix",
                    "client",
                    "v3",
                    "rooms",
                    room.as_str(),
                    "state",
                    AGENT_ROSTER_POLICY_EVENT_TYPE,
                    "",
                ],
                operation,
            )?;
            let response = self.client.put(endpoint).bearer_auth(self.access_token.expose())
                .json(&serde_json::json!({ "schemaVersion": 1, "archiveAfterDays": policy.archive_after_days() }))
                .send().await.map_err(|error| super::map_transport_error(operation, &error))?;
            expect_empty_success(response, operation).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_room_application::ports::SecretValue;
    use axum::{Json, Router, extract::Path, http::HeaderMap, routing::put};
    use serde_json::{Value, json};
    use std::time::Duration;

    #[tokio::test]
    async fn 归档规则通过受控状态端点发布且拒绝响应不冒充成功() {
        let app = Router::new().route(
            "/_matrix/client/v3/rooms/{room}/state/io.github.rainyflash.agentroom.roster.policy.v1/",
            put(|Path(room): Path<String>, headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(room, "!managed:matrix.test");
                assert_eq!(headers.get("authorization").unwrap(), "Bearer test-as-secret");
                assert_eq!(body["schemaVersion"], 1);
                if body["archiveAfterDays"] == 30 {
                    (axum::http::StatusCode::OK, Json(json!({"event_id": "$policy:matrix.test"})))
                } else {
                    (axum::http::StatusCode::FORBIDDEN, Json(json!({"errcode": "M_FORBIDDEN"})))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = MatrixApplicationServiceProvisioner::new(
            crate::MatrixApplicationServiceConfiguration::new(
                url,
                "matrix.test",
                SecretValue::new("test-as-secret").unwrap(),
                Duration::from_secs(2),
            )
            .unwrap(),
        )
        .unwrap();
        let room = MatrixRoomId::new("!managed:matrix.test").unwrap();
        let saved = client
            .publish(&room, AgentRosterPolicy::new(30).unwrap())
            .await;
        let rejected = client
            .publish(&room, AgentRosterPolicy::new(1).unwrap())
            .await;
        server.abort();
        assert!(saved.is_ok());
        assert!(rejected.is_err());
    }
}
