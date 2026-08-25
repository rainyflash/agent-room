use agent_room_application::ports::{
    MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, ModerationEffectGateway,
    ModerationEffectTarget, PortFuture, PrivateRoomMatrixGateway,
};
use agent_room_domain::moderation::{ModerationAction, ModerationActionKind};
use serde_json::json;

use crate::{
    MatrixApplicationServiceProvisioner,
    rooms::{endpoint_with_segments, expect_empty_success},
};

const MODERATION_NOTICE_EVENT_TYPE: &str = "org.agentroom.moderation.notice.v1";

impl ModerationEffectGateway for MatrixApplicationServiceProvisioner {
    fn apply<'a>(
        &'a self,
        action: &'a ModerationAction,
        target: &'a ModerationEffectTarget,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(apply_effect(self, action, target))
    }

    fn reverse<'a>(
        &'a self,
        action: &'a ModerationAction,
        target: &'a ModerationEffectTarget,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(reverse_effect(self, action, target))
    }
}

async fn apply_effect(
    provisioner: &MatrixApplicationServiceProvisioner,
    action: &ModerationAction,
    target: &ModerationEffectTarget,
) -> MatrixResult<()> {
    validate_effect_target(action, target)?;
    match action.kind() {
        ModerationActionKind::Hide => {
            write_moderation_notice(provisioner, action, target, true).await
        }
        ModerationActionKind::Mute => {
            PrivateRoomMatrixGateway::set_speaking(
                provisioner,
                &target.matrix_room_id,
                required_matrix_user(target)?,
                false,
            )
            .await
        }
        ModerationActionKind::Kick => {
            PrivateRoomMatrixGateway::kick(
                provisioner,
                &target.matrix_room_id,
                required_matrix_user(target)?,
            )
            .await
        }
        ModerationActionKind::Ban => {
            PrivateRoomMatrixGateway::ban(
                provisioner,
                &target.matrix_room_id,
                required_matrix_user(target)?,
            )
            .await
        }
    }
}

async fn reverse_effect(
    provisioner: &MatrixApplicationServiceProvisioner,
    action: &ModerationAction,
    target: &ModerationEffectTarget,
) -> MatrixResult<()> {
    validate_effect_target(action, target)?;
    match action.kind() {
        ModerationActionKind::Hide => {
            write_moderation_notice(provisioner, action, target, false).await
        }
        ModerationActionKind::Mute => {
            PrivateRoomMatrixGateway::set_speaking(
                provisioner,
                &target.matrix_room_id,
                required_matrix_user(target)?,
                true,
            )
            .await
        }
        ModerationActionKind::Kick => {
            PrivateRoomMatrixGateway::invite(
                provisioner,
                &target.matrix_room_id,
                required_matrix_user(target)?,
            )
            .await
        }
        ModerationActionKind::Ban => {
            let user_id = required_matrix_user(target)?;
            unban(provisioner, target, user_id).await?;
            PrivateRoomMatrixGateway::invite(provisioner, &target.matrix_room_id, user_id).await
        }
    }
}

fn validate_effect_target(
    action: &ModerationAction,
    target: &ModerationEffectTarget,
) -> MatrixResult<()> {
    if action.target() != &target.target {
        return Err(invalid_configuration());
    }
    Ok(())
}

fn required_matrix_user(
    target: &ModerationEffectTarget,
) -> MatrixResult<&agent_room_application::ports::MatrixUserId> {
    target
        .target_matrix_user_id
        .as_ref()
        .ok_or_else(invalid_configuration)
}

async fn unban(
    provisioner: &MatrixApplicationServiceProvisioner,
    target: &ModerationEffectTarget,
    user_id: &agent_room_application::ports::MatrixUserId,
) -> MatrixResult<()> {
    let operation = MatrixOperation::Unban;
    let endpoint = endpoint_with_segments(
        &provisioner.homeserver_url,
        &[
            "_matrix",
            "client",
            "v3",
            "rooms",
            target.matrix_room_id.as_str(),
            "unban",
        ],
        operation,
    )?;
    let response = provisioner
        .client
        .post(endpoint)
        .bearer_auth(provisioner.access_token.expose())
        .json(&json!({ "user_id": user_id.as_str() }))
        .send()
        .await
        .map_err(|error| super::map_transport_error(operation, &error))?;
    expect_empty_success(response, operation).await
}

async fn write_moderation_notice(
    provisioner: &MatrixApplicationServiceProvisioner,
    action: &ModerationAction,
    target: &ModerationEffectTarget,
    hidden: bool,
) -> MatrixResult<()> {
    let operation = MatrixOperation::SendStateEvent;
    let endpoint = endpoint_with_segments(
        &provisioner.homeserver_url,
        &[
            "_matrix",
            "client",
            "v3",
            "rooms",
            target.matrix_room_id.as_str(),
            "state",
            MODERATION_NOTICE_EVENT_TYPE,
            action.target().reference(),
        ],
        operation,
    )?;
    let response = provisioner
        .client
        .put(endpoint)
        .bearer_auth(provisioner.access_token.expose())
        .json(&json!({
            "schemaVersion": "1.0",
            "eventType": MODERATION_NOTICE_EVENT_TYPE,
            "actionId": action.id().to_string(),
            "targetEventId": action.target().reference(),
            "hidden": hidden,
            "reasonCode": action.reason().as_str()
        }))
        .send()
        .await
        .map_err(|error| super::map_transport_error(operation, &error))?;
    expect_empty_success(response, operation).await
}

const fn invalid_configuration() -> MatrixFailure {
    MatrixFailure::new(
        MatrixOperation::InspectRoomAuthority,
        MatrixFailureKind::InvalidConfiguration,
    )
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc, time::Duration};

    use agent_room_application::ports::{
        MatrixRoomId, MatrixUserId, ModerationEffectGateway, ModerationEffectTarget, SecretValue,
    };
    use agent_room_domain::{
        ids::{ModerationActionId, PrincipalId, RoomCatalogId},
        moderation::{
            ModerationAction, ModerationActionKind, ModerationReason, ModerationTarget,
            ModerationTargetKind,
        },
        time::UtcMillis,
    };
    use axum::{
        Json, Router,
        extract::{Path, State},
        http::HeaderMap,
        routing::{get, post, put},
    };
    use serde_json::{Value, json};
    use tokio::{net::TcpListener, task::JoinHandle};
    use uuid::Uuid;

    use crate::{MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner};

    #[tokio::test]
    async fn 四类治理动作与撤销都落到可重试_matrix_端点() {
        let server = ModerationTestServer::start().await;
        let provisioner = provisioner(&server.url);
        let room = MatrixRoomId::new("!governed:matrix.agent-room.localhost").expect("房间有效");
        let principal = PrincipalId::from_uuid(Uuid::now_v7());
        let matrix_user =
            MatrixUserId::new("@member:matrix.agent-room.localhost").expect("成员有效");
        server.join(matrix_user.as_str()).await;

        for kind in [
            ModerationActionKind::Mute,
            ModerationActionKind::Kick,
            ModerationActionKind::Ban,
        ] {
            let action = action(kind, principal);
            let target = ModerationEffectTarget {
                matrix_room_id: room.clone(),
                target: action.target().clone(),
                target_matrix_user_id: Some(matrix_user.clone()),
            };
            ModerationEffectGateway::apply(&provisioner, &action, &target)
                .await
                .expect("治理副作用应成功");
            ModerationEffectGateway::reverse(&provisioner, &action, &target)
                .await
                .expect("治理撤销应成功");
            server.join(matrix_user.as_str()).await;
        }

        let hide = hide_action(principal);
        let hide_target = ModerationEffectTarget {
            matrix_room_id: room,
            target: hide.target().clone(),
            target_matrix_user_id: None,
        };
        ModerationEffectGateway::apply(&provisioner, &hide, &hide_target)
            .await
            .expect("隐藏通知应成功");
        ModerationEffectGateway::reverse(&provisioner, &hide, &hide_target)
            .await
            .expect("取消隐藏应成功");

        let calls = server.calls().await;
        assert!(calls.iter().any(|call| call == "kick"));
        assert!(calls.iter().any(|call| call == "ban"));
        assert!(calls.iter().any(|call| call == "unban"));
        assert!(calls.iter().filter(|call| *call == "invite").count() >= 2);
        assert_eq!(
            calls.iter().filter(|call| *call == "write-power").count(),
            2
        );
        let notices = server.notices().await;
        assert_eq!(notices.len(), 2);
        assert_eq!(notices[0]["hidden"], true);
        assert_eq!(notices[1]["hidden"], false);
        assert!(notices.iter().all(|notice| notice.get("body").is_none()));
    }

    #[tokio::test]
    async fn matrix_适配器拒绝动作与目标偷换() {
        let provisioner = provisioner("http://127.0.0.1:9");
        let principal = PrincipalId::from_uuid(Uuid::now_v7());
        let action = action(ModerationActionKind::Mute, principal);
        let target = ModerationEffectTarget {
            matrix_room_id: MatrixRoomId::new("!room:matrix.agent-room.localhost")
                .expect("房间有效"),
            target: ModerationTarget::new(
                ModerationTargetKind::Principal,
                Uuid::now_v7().to_string(),
            )
            .expect("另一目标有效"),
            target_matrix_user_id: Some(
                MatrixUserId::new("@member:matrix.agent-room.localhost").expect("成员有效"),
            ),
        };

        let failure = ModerationEffectGateway::apply(&provisioner, &action, &target)
            .await
            .expect_err("目标偷换必须在联网前拒绝");
        assert_eq!(
            failure.kind(),
            agent_room_application::ports::MatrixFailureKind::InvalidConfiguration
        );
    }

    fn action(kind: ModerationActionKind, principal: PrincipalId) -> ModerationAction {
        ModerationAction::reserve(
            ModerationActionId::from_uuid(Uuid::now_v7()),
            None,
            principal,
            RoomCatalogId::from_uuid(Uuid::now_v7()),
            kind,
            ModerationTarget::new(ModerationTargetKind::Principal, principal.to_string())
                .expect("主体目标有效"),
            ModerationReason::Harassment,
            UtcMillis::new(1_800_000_000_000).expect("时间有效"),
            None,
        )
        .expect("治理动作有效")
    }

    fn hide_action(principal: PrincipalId) -> ModerationAction {
        ModerationAction::reserve(
            ModerationActionId::from_uuid(Uuid::now_v7()),
            None,
            principal,
            RoomCatalogId::from_uuid(Uuid::now_v7()),
            ModerationActionKind::Hide,
            ModerationTarget::new(ModerationTargetKind::Event, "$event:matrix.test")
                .expect("事件目标有效"),
            ModerationReason::MaliciousContent,
            UtcMillis::new(1_800_000_000_000).expect("时间有效"),
            None,
        )
        .expect("隐藏动作有效")
    }

    fn provisioner(url: &str) -> MatrixApplicationServiceProvisioner {
        MatrixApplicationServiceProvisioner::new(
            MatrixApplicationServiceConfiguration::new(
                url,
                "matrix.agent-room.localhost",
                SecretValue::new("application-service-secret").expect("密钥有效"),
                Duration::from_secs(2),
            )
            .expect("配置有效"),
        )
        .expect("适配器有效")
    }

    struct ModerationTestServer {
        url: String,
        state: Arc<TestState>,
        task: JoinHandle<()>,
    }

    #[derive(Default)]
    struct TestState {
        calls: tokio::sync::Mutex<Vec<String>>,
        memberships: tokio::sync::Mutex<BTreeMap<String, String>>,
        notices: tokio::sync::Mutex<Vec<Value>>,
    }

    impl ModerationTestServer {
        async fn start() -> Self {
            let state = Arc::new(TestState::default());
            let app = Router::new()
                .route(
                    "/_matrix/client/v3/rooms/{room}/state/m.room.member/{user}",
                    get(read_membership),
                )
                .route(
                    "/_matrix/client/v3/rooms/{room}/state/m.room.power_levels",
                    get(read_power),
                )
                .route(
                    "/_matrix/client/v3/rooms/{room}/state/m.room.power_levels/",
                    put(write_power),
                )
                .route(
                    "/_matrix/client/v3/rooms/{room}/state/org.agentroom.moderation.notice.v1/{event}",
                    put(write_notice),
                )
                .route(
                    "/_matrix/client/v3/rooms/{room}/{action}",
                    post(change_membership),
                )
                .with_state(state.clone());
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("测试端口可用");
            let address = listener.local_addr().expect("测试地址有效");
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.expect("测试服务可运行");
            });
            Self {
                url: format!("http://{address}"),
                state,
                task,
            }
        }

        async fn join(&self, user: &str) {
            self.state
                .memberships
                .lock()
                .await
                .insert(user.to_owned(), "join".to_owned());
        }

        async fn calls(&self) -> Vec<String> {
            self.state.calls.lock().await.clone()
        }

        async fn notices(&self) -> Vec<Value> {
            self.state.notices.lock().await.clone()
        }
    }

    impl Drop for ModerationTestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    fn assert_authentication(headers: &HeaderMap) {
        assert_eq!(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer application-service-secret")
        );
    }

    async fn read_membership(
        State(state): State<Arc<TestState>>,
        Path((_room, user)): Path<(String, String)>,
        headers: HeaderMap,
    ) -> Json<Value> {
        assert_authentication(&headers);
        state.calls.lock().await.push("membership".to_owned());
        let membership = state
            .memberships
            .lock()
            .await
            .get(&user)
            .cloned()
            .unwrap_or_else(|| "leave".to_owned());
        Json(json!({ "membership": membership }))
    }

    async fn change_membership(
        State(state): State<Arc<TestState>>,
        Path((_room, action)): Path<(String, String)>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        assert_authentication(&headers);
        let user = body["user_id"].as_str().expect("成员标识存在").to_owned();
        let membership = match action.as_str() {
            "kick" | "unban" => "leave",
            "ban" => "ban",
            "invite" => "invite",
            unexpected => panic!("未知成员动作 {unexpected}"),
        };
        state
            .memberships
            .lock()
            .await
            .insert(user, membership.to_owned());
        state.calls.lock().await.push(action);
        Json(json!({}))
    }

    async fn read_power(State(state): State<Arc<TestState>>, headers: HeaderMap) -> Json<Value> {
        assert_authentication(&headers);
        state.calls.lock().await.push("read-power".to_owned());
        Json(json!({ "users": {} }))
    }

    async fn write_power(
        State(state): State<Arc<TestState>>,
        headers: HeaderMap,
        Json(_body): Json<Value>,
    ) -> Json<Value> {
        assert_authentication(&headers);
        state.calls.lock().await.push("write-power".to_owned());
        Json(json!({ "event_id": "$power" }))
    }

    async fn write_notice(
        State(state): State<Arc<TestState>>,
        Path((_room, event)): Path<(String, String)>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        assert_authentication(&headers);
        assert_eq!(event, "$event:matrix.test");
        state.calls.lock().await.push("notice".to_owned());
        state.notices.lock().await.push(body);
        Json(json!({ "event_id": "$notice" }))
    }
}
