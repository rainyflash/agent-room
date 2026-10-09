use agent_room_application::ports::{
    MatrixEventId, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
    MatrixUserId, ModerationEffectGateway, ModerationEffectTarget, PortFuture,
    PrivateMatrixMembership, PrivateRoomMatrixGateway,
};
use agent_room_domain::{
    moderation::{ModerationAction, ModerationActionKind},
    rooms::RoomCatalogKind,
};
use reqwest::StatusCode;
use serde_json::{Map, Value, json};

use crate::{
    MatrixApplicationServiceProvisioner, decode_matrix_error, invalid_response, map_matrix_error,
    read_limited_body,
    rooms::{
        endpoint_with_segments, expect_empty_success, read_power_levels,
        write_power_levels_if_changed,
    },
};

const MODERATION_NOTICE_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.moderation.notice.v1";

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

    fn contains_event<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, MatrixResult<bool>> {
        Box::pin(contains_event(self, room_id, event_id))
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
        ModerationActionKind::Mute => set_speaking(provisioner, target, false).await,
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
        ModerationActionKind::Mute => set_speaking(provisioner, target, true).await,
        // 公开大厅谁都能进：被踢出的人自己回来就行，不在每个分片里发邀请。
        ModerationActionKind::Kick if target.room_kind == RoomCatalogKind::PublicLobby => Ok(()),
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
            if target.room_kind == RoomCatalogKind::PublicLobby {
                return Ok(());
            }
            PrivateRoomMatrixGateway::invite(provisioner, &target.matrix_room_id, user_id).await
        }
    }
}

/// 禁言或撤销禁言。私人房间按成员发言权那一套（发言级别就是 `events_default`）。公开大厅谁都能
/// 说话，不能套私人房间那套：那会把整个大厅的门槛抬到发言级别，谁都说不了话。这里只把这个人压到
/// 门槛以下，撤销时去掉这一项。
async fn set_speaking(
    provisioner: &MatrixApplicationServiceProvisioner,
    target: &ModerationEffectTarget,
    allowed: bool,
) -> MatrixResult<()> {
    let user_id = required_matrix_user(target)?;
    if target.room_kind != RoomCatalogKind::PublicLobby {
        return PrivateRoomMatrixGateway::set_speaking(
            provisioner,
            &target.matrix_room_id,
            user_id,
            allowed,
        )
        .await;
    }
    let operation = MatrixOperation::UpdatePowerLevels;
    let original = read_power_levels(provisioner, &target.matrix_room_id, operation).await?;
    let mut content = original.clone();
    set_public_speaker(&mut content, user_id, allowed, operation)?;
    write_power_levels_if_changed(
        provisioner,
        &target.matrix_room_id,
        &original,
        &content,
        operation,
    )
    .await
}

/// 公开大厅的消息事件都没单列级别，发言门槛就是 `events_default`。禁言把这个人压到门槛下一级；
/// 撤销时只去掉禁言压下去的那一项，回到 `users_default`。门槛和别人的级别都不动，已经是这样的
/// 不再写。
fn set_public_speaker(
    content: &mut Map<String, Value>,
    user_id: &MatrixUserId,
    allowed: bool,
    operation: MatrixOperation,
) -> MatrixResult<()> {
    let threshold = power_level(content.get("events_default"), operation)?.unwrap_or(0);
    let users_default = power_level(content.get("users_default"), operation)?.unwrap_or(0);
    let current = match content.get("users") {
        None => None,
        Some(users) => power_level(
            users
                .as_object()
                .ok_or_else(|| invalid_response(operation))?
                .get(user_id.as_str()),
            operation,
        )?,
    };
    if allowed {
        if current.is_some_and(|level| level < threshold)
            && let Some(users) = content.get_mut("users").and_then(Value::as_object_mut)
        {
            users.remove(user_id.as_str());
        }
        return Ok(());
    }
    if current.unwrap_or(users_default) < threshold {
        return Ok(());
    }
    content
        .entry("users".to_owned())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| invalid_response(operation))?
        .insert(
            user_id.as_str().to_owned(),
            Value::from(threshold.saturating_sub(1)),
        );
    Ok(())
}

fn power_level(value: Option<&Value>, operation: MatrixOperation) -> MatrixResult<Option<i64>> {
    value
        .map(|value| value.as_i64().ok_or_else(|| invalid_response(operation)))
        .transpose()
}

/// 这个房间里有没有这条事件：以建房间的应用服务账号读，不冒充任何人。事件不在这个房间、或者
/// 读不到，Synapse 一律回 404。只看状态码，不读事件正文：一条事件最大 64 KiB，比这里收回答的上限大。
async fn contains_event(
    provisioner: &MatrixApplicationServiceProvisioner,
    room_id: &MatrixRoomId,
    event_id: &MatrixEventId,
) -> MatrixResult<bool> {
    let operation = MatrixOperation::ReadRoomEvent;
    let endpoint = endpoint_with_segments(
        &provisioner.homeserver_url,
        &[
            "_matrix",
            "client",
            "v3",
            "rooms",
            room_id.as_str(),
            "event",
            event_id.as_str(),
        ],
        operation,
    )?;
    let response = provisioner
        .client
        .get(endpoint)
        .bearer_auth(provisioner.access_token.expose())
        .send()
        .await
        .map_err(|error| super::map_transport_error(operation, &error))?;
    let status = response.status();
    if status.is_success() {
        return Ok(true);
    }
    if status == StatusCode::NOT_FOUND {
        return Ok(false);
    }
    let body = read_limited_body(response, operation).await?;
    let error = decode_matrix_error(&body, operation)?;
    Err(map_matrix_error(operation, status, &error))
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

fn required_matrix_user(target: &ModerationEffectTarget) -> MatrixResult<&MatrixUserId> {
    target
        .target_matrix_user_id
        .as_ref()
        .ok_or_else(invalid_configuration)
}

/// 只解这个分片里真封着的人：Synapse 不让解没封的人（`M_BAD_STATE`），封禁之后新开的分片里他
/// 本来就没被封。
async fn unban(
    provisioner: &MatrixApplicationServiceProvisioner,
    target: &ModerationEffectTarget,
    user_id: &MatrixUserId,
) -> MatrixResult<()> {
    if PrivateRoomMatrixGateway::membership(provisioner, &target.matrix_room_id, user_id).await?
        != Some(PrivateMatrixMembership::Banned)
    {
        return Ok(());
    }
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
        MatrixEventId, MatrixFailureKind, MatrixOperation, MatrixRoomId, MatrixUserId,
        ModerationEffectGateway, ModerationEffectTarget, SecretValue,
    };
    use agent_room_domain::{
        ids::{ModerationActionId, PrincipalId, RoomCatalogId},
        moderation::{
            ModerationAction, ModerationActionKind, ModerationReason, ModerationTarget,
            ModerationTargetKind,
        },
        rooms::RoomCatalogKind,
        time::UtcMillis,
    };
    use axum::{
        Json, Router,
        extract::{Path, RawQuery, State},
        http::{HeaderMap, StatusCode},
        routing::{get, post, put},
    };
    use serde_json::{Value, json};
    use tokio::{net::TcpListener, task::JoinHandle};
    use uuid::Uuid;

    use crate::{MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner};

    const MEMBER: &str = "@member:matrix.agent-room.localhost";
    const SERVICE: &str = "@agent-room:matrix.agent-room.localhost";

    #[tokio::test]
    async fn 四类治理动作与撤销都落到可重试_matrix_端点() {
        let server = ModerationTestServer::start().await;
        let provisioner = provisioner(&server.url);
        let room = MatrixRoomId::new("!governed:matrix.agent-room.localhost").expect("房间有效");
        let principal = PrincipalId::from_uuid(Uuid::now_v7());
        let matrix_user = MatrixUserId::new(MEMBER).expect("成员有效");
        server.join(matrix_user.as_str()).await;

        for kind in [
            ModerationActionKind::Mute,
            ModerationActionKind::Kick,
            ModerationActionKind::Ban,
        ] {
            let action = action(kind, principal);
            let target = person_target(&room, RoomCatalogKind::PrivateRoom, &action);
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
            room_kind: RoomCatalogKind::PrivateRoom,
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
    async fn 公开大厅禁言只压低这个人_不动门槛_撤销时去掉这一项() {
        let server = ModerationTestServer::start().await;
        let provisioner = provisioner(&server.url);
        let room = MatrixRoomId::new("!lobby:matrix.agent-room.localhost").expect("房间有效");
        let lobby_levels = json!({
            "users": { SERVICE: 100 },
            "users_default": 0,
            "events_default": 0,
            "state_default": 50,
            "events": { "io.github.rainyflash.agentroom.agent.status.v1": 0 }
        });
        server.set_power_levels(lobby_levels.clone()).await;
        let action = action(
            ModerationActionKind::Mute,
            PrincipalId::from_uuid(Uuid::now_v7()),
        );
        let target = person_target(&room, RoomCatalogKind::PublicLobby, &action);

        for _ in 0..2 {
            ModerationEffectGateway::apply(&provisioner, &action, &target)
                .await
                .expect("禁言应成功");
        }
        let writes = server.power_writes().await;
        assert_eq!(writes.len(), 1, "已经禁言的不再写");
        let mut muted = lobby_levels.clone();
        muted["users"][MEMBER] = json!(-1);
        assert_eq!(writes[0], muted, "只压低这个人，门槛和别人都不动");

        for _ in 0..2 {
            ModerationEffectGateway::reverse(&provisioner, &action, &target)
                .await
                .expect("撤销禁言应成功");
        }
        let writes = server.power_writes().await;
        assert_eq!(writes.len(), 2, "已经撤销的不再写");
        assert_eq!(writes[1], lobby_levels, "撤销时去掉这一项，回到原来的样子");
    }

    #[tokio::test]
    async fn 公开大厅撤销踢出不发邀请_撤销封禁只解封_没封着的分片不碰() {
        let server = ModerationTestServer::start().await;
        let provisioner = provisioner(&server.url);
        let room = MatrixRoomId::new("!lobby:matrix.agent-room.localhost").expect("房间有效");
        let principal = PrincipalId::from_uuid(Uuid::now_v7());
        server.join(MEMBER).await;

        let kick = action(ModerationActionKind::Kick, principal);
        let kick_target = person_target(&room, RoomCatalogKind::PublicLobby, &kick);
        ModerationEffectGateway::apply(&provisioner, &kick, &kick_target)
            .await
            .expect("踢出应成功");
        ModerationEffectGateway::reverse(&provisioner, &kick, &kick_target)
            .await
            .expect("撤销踢出应成功");
        assert_eq!(server.take_calls().await, ["membership", "kick"]);

        let ban = action(ModerationActionKind::Ban, principal);
        let ban_target = person_target(&room, RoomCatalogKind::PublicLobby, &ban);
        ModerationEffectGateway::apply(&provisioner, &ban, &ban_target)
            .await
            .expect("封禁应成功");
        // 封着的人不再踢：他已经不在房间里，Synapse 也不让踢。
        ModerationEffectGateway::apply(&provisioner, &kick, &kick_target)
            .await
            .expect("踢已经封禁的人什么也不做");
        ModerationEffectGateway::reverse(&provisioner, &ban, &ban_target)
            .await
            .expect("撤销封禁应成功");
        assert_eq!(
            server.take_calls().await,
            ["membership", "ban", "membership", "membership", "unban"]
        );

        // 封禁之后新开的分片里他本来就没被封：撤销时不去解。
        ModerationEffectGateway::reverse(&provisioner, &ban, &ban_target)
            .await
            .expect("没封着的分片里撤销封禁什么也不做");
        assert_eq!(server.take_calls().await, ["membership"]);
    }

    #[tokio::test]
    async fn 按事件_id_找分片_读到是有_404_是没有_别的错误照实报() {
        let server = ModerationTestServer::start().await;
        let provisioner = provisioner(&server.url);
        let busy = MatrixRoomId::new("!busy:matrix.agent-room.localhost").expect("房间有效");
        let quiet = MatrixRoomId::new("!quiet:matrix.agent-room.localhost").expect("房间有效");
        let event = MatrixEventId::new("$spam:matrix.agent-room.localhost").expect("事件有效");
        server.put_event(quiet.as_str(), event.as_str()).await;

        assert!(
            !ModerationEffectGateway::contains_event(&provisioner, &busy, &event)
                .await
                .expect("不在这个分片里是 404，不是错误")
        );
        // 读到的事件可能比收回答的上限还大：只看状态码。
        assert!(
            ModerationEffectGateway::contains_event(&provisioner, &quiet, &event)
                .await
                .expect("这个分片里有")
        );
        assert_eq!(
            server.event_reads().await,
            [
                (busy.as_str().to_owned(), None),
                (quiet.as_str().to_owned(), None)
            ],
            "以应用服务自己的账号读，不冒充任何人"
        );

        server.break_room(busy.as_str()).await;
        let failure = ModerationEffectGateway::contains_event(&provisioner, &busy, &event)
            .await
            .expect_err("读不了要照实报，不能当成没有");
        assert_eq!(failure.kind(), MatrixFailureKind::Forbidden);
        assert_eq!(failure.operation(), MatrixOperation::ReadRoomEvent);
    }

    #[tokio::test]
    async fn matrix_适配器拒绝动作与目标偷换() {
        let provisioner = provisioner("http://127.0.0.1:9");
        let principal = PrincipalId::from_uuid(Uuid::now_v7());
        let action = action(ModerationActionKind::Mute, principal);
        let target = ModerationEffectTarget {
            matrix_room_id: MatrixRoomId::new("!room:matrix.agent-room.localhost")
                .expect("房间有效"),
            room_kind: RoomCatalogKind::PrivateRoom,
            target: ModerationTarget::new(
                ModerationTargetKind::Principal,
                Uuid::now_v7().to_string(),
            )
            .expect("另一目标有效"),
            target_matrix_user_id: Some(MatrixUserId::new(MEMBER).expect("成员有效")),
        };

        let failure = ModerationEffectGateway::apply(&provisioner, &action, &target)
            .await
            .expect_err("目标偷换必须在联网前拒绝");
        assert_eq!(failure.kind(), MatrixFailureKind::InvalidConfiguration);
    }

    fn person_target(
        room: &MatrixRoomId,
        room_kind: RoomCatalogKind,
        action: &ModerationAction,
    ) -> ModerationEffectTarget {
        ModerationEffectTarget {
            matrix_room_id: room.clone(),
            room_kind,
            target: action.target().clone(),
            target_matrix_user_id: Some(MatrixUserId::new(MEMBER).expect("成员有效")),
        }
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
        /// 房间此刻的权限状态；没设过时是空的 `users`。
        power_levels: tokio::sync::Mutex<Option<Value>>,
        power_writes: tokio::sync::Mutex<Vec<Value>>,
        /// 事件 ID 到它所在的房间。
        events: tokio::sync::Mutex<BTreeMap<String, String>>,
        /// 每次按事件 ID 读：房间和查询串。
        event_reads: tokio::sync::Mutex<Vec<(String, Option<String>)>>,
        broken_room: tokio::sync::Mutex<Option<String>>,
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
                    "/_matrix/client/v3/rooms/{room}/state/io.github.rainyflash.agentroom.moderation.notice.v1/{event}",
                    put(write_notice),
                )
                .route(
                    "/_matrix/client/v3/rooms/{room}/event/{event}",
                    get(read_event),
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

        async fn set_power_levels(&self, levels: Value) {
            *self.state.power_levels.lock().await = Some(levels);
        }

        async fn put_event(&self, room: &str, event: &str) {
            self.state
                .events
                .lock()
                .await
                .insert(event.to_owned(), room.to_owned());
        }

        async fn break_room(&self, room: &str) {
            *self.state.broken_room.lock().await = Some(room.to_owned());
        }

        async fn calls(&self) -> Vec<String> {
            self.state.calls.lock().await.clone()
        }

        async fn take_calls(&self) -> Vec<String> {
            std::mem::take(&mut *self.state.calls.lock().await)
        }

        async fn notices(&self) -> Vec<Value> {
            self.state.notices.lock().await.clone()
        }

        async fn power_writes(&self) -> Vec<Value> {
            self.state.power_writes.lock().await.clone()
        }

        async fn event_reads(&self) -> Vec<(String, Option<String>)> {
            self.state.event_reads.lock().await.clone()
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
        let levels = state.power_levels.lock().await.clone();
        Json(levels.unwrap_or_else(|| json!({ "users": {} })))
    }

    async fn write_power(
        State(state): State<Arc<TestState>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        assert_authentication(&headers);
        state.calls.lock().await.push("write-power".to_owned());
        *state.power_levels.lock().await = Some(body.clone());
        state.power_writes.lock().await.push(body);
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

    async fn read_event(
        State(state): State<Arc<TestState>>,
        Path((room, event)): Path<(String, String)>,
        RawQuery(query): RawQuery,
        headers: HeaderMap,
    ) -> (StatusCode, Json<Value>) {
        assert_authentication(&headers);
        state.event_reads.lock().await.push((room.clone(), query));
        if state.broken_room.lock().await.as_deref() == Some(room.as_str()) {
            return (
                StatusCode::FORBIDDEN,
                Json(
                    json!({ "errcode": "M_FORBIDDEN", "error": "Application service has not registered this user" }),
                ),
            );
        }
        if state.events.lock().await.get(&event) != Some(&room) {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "errcode": "M_NOT_FOUND", "error": "Event not found." })),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "event_id": event,
                "room_id": room,
                "sender": MEMBER,
                "type": "io.github.rainyflash.agentroom.message.preview.v2",
                "content": { "body": "x".repeat(40_000) }
            })),
        )
    }
}
