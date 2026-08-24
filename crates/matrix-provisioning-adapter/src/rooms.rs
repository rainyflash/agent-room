use std::{fmt, sync::Arc};

use agent_room_application::ports::{
    AgentRoomMembershipFactory, MatrixCreateRoom, MatrixEventId, MatrixFailure, MatrixFailureKind,
    MatrixOperation, MatrixResult, MatrixRoomAliasLocalpart, MatrixRoomId, MatrixRoomKind,
    MatrixRoomPreset, MatrixRoomVisibility, MatrixUserId, PortFuture, RoomMembershipGateway,
    RoomProvisioningGateway,
};
use agent_room_domain::rooms::MatrixRoomReference;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    MatrixApplicationServiceProvisioner, decode_json, decode_matrix_error, map_matrix_error,
    map_transport_error, read_limited_body,
};

/// 仅能代表一个已验证的受管 Agent 用户执行入房和退房。
#[derive(Clone)]
pub struct MatrixApplicationServiceRoomMembership {
    application_service: Arc<MatrixApplicationServiceProvisioner>,
    user_id: MatrixUserId,
}

impl MatrixApplicationServiceRoomMembership {
    pub(crate) const fn new(
        application_service: Arc<MatrixApplicationServiceProvisioner>,
        user_id: MatrixUserId,
    ) -> Self {
        Self {
            application_service,
            user_id,
        }
    }

    async fn change_membership(
        &self,
        room_id: &MatrixRoomReference,
        action: &'static str,
        operation: MatrixOperation,
    ) -> MatrixResult<()> {
        let mut endpoint = endpoint_with_segments(
            &self.application_service.homeserver_url,
            &["_matrix", "client", "v3", "rooms", room_id.as_str(), action],
            operation,
        )?;
        endpoint
            .query_pairs_mut()
            .append_pair("user_id", self.user_id.as_str());
        let response = self
            .application_service
            .client
            .post(endpoint)
            .bearer_auth(self.application_service.access_token.expose())
            .json(&EmptyRequest {})
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        expect_empty_success(response, operation).await
    }
}

impl fmt::Debug for MatrixApplicationServiceRoomMembership {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixApplicationServiceRoomMembership")
            .field("user_id", &self.user_id)
            .finish_non_exhaustive()
    }
}

impl RoomMembershipGateway for MatrixApplicationServiceRoomMembership {
    fn join<'a>(&'a self, room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(self.change_membership(room_id, "join", MatrixOperation::Join))
    }

    fn leave<'a>(&'a self, room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(self.change_membership(room_id, "leave", MatrixOperation::Leave))
    }
}

impl AgentRoomMembershipFactory for MatrixApplicationServiceProvisioner {
    fn bind(&self, matrix_user_id: &MatrixUserId) -> MatrixResult<Arc<dyn RoomMembershipGateway>> {
        self.room_membership(matrix_user_id.clone())
            .map(|membership| Arc::new(membership) as Arc<dyn RoomMembershipGateway>)
    }
}

impl RoomProvisioningGateway for MatrixApplicationServiceProvisioner {
    fn create_room<'a>(
        &'a self,
        request: &'a MatrixCreateRoom,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(async move {
            let operation = MatrixOperation::CreateRoom;
            let response = self
                .client
                .post(self.endpoint("_matrix/client/v3/createRoom", operation)?)
                .bearer_auth(self.access_token.expose())
                .json(&CreateRoomRequest::from(request))
                .send()
                .await
                .map_err(|error| map_create_transport_error(operation, &error))?;
            let body = expect_success_body(response, operation).await?;
            let created: CreateRoomResponse = decode_json(&body, operation)?;
            MatrixRoomId::new(created.room_id).map_err(|_| invalid_response(operation))
        })
    }

    fn resolve_room_alias<'a>(
        &'a self,
        alias_localpart: &'a MatrixRoomAliasLocalpart,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(async move {
            let operation = MatrixOperation::ResolveRoomAlias;
            let alias = format!("#{}:{}", alias_localpart.as_str(), self.server_name);
            let endpoint = endpoint_with_segments(
                &self.homeserver_url,
                &["_matrix", "client", "v3", "directory", "room", &alias],
                operation,
            )?;
            let response = self
                .client
                .get(endpoint)
                .bearer_auth(self.access_token.expose())
                .send()
                .await
                .map_err(|error| map_transport_error(operation, &error))?;
            let body = expect_success_body(response, operation).await?;
            let resolved: ResolveRoomAliasResponse = decode_json(&body, operation)?;
            MatrixRoomId::new(resolved.room_id).map_err(|_| invalid_response(operation))
        })
    }

    fn attach_child<'a>(
        &'a self,
        space_id: &'a MatrixRoomId,
        child_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        Box::pin(async move {
            let operation = MatrixOperation::SendStateEvent;
            let child_server = matrix_server_name(child_id.as_str(), operation)?;
            let endpoint = endpoint_with_segments(
                &self.homeserver_url,
                &[
                    "_matrix",
                    "client",
                    "v3",
                    "rooms",
                    space_id.as_str(),
                    "state",
                    "m.space.child",
                    child_id.as_str(),
                ],
                operation,
            )?;
            let response = self
                .client
                .put(endpoint)
                .bearer_auth(self.access_token.expose())
                .json(&json!({ "via": [child_server], "suggested": true }))
                .send()
                .await
                .map_err(|error| map_transport_error(operation, &error))?;
            let body = expect_success_body(response, operation).await?;
            let accepted: StateEventResponse = decode_json(&body, operation)?;
            MatrixEventId::new(accepted.event_id).map_err(|_| invalid_response(operation))
        })
    }
}

#[derive(Serialize)]
struct EmptyRequest {}

#[derive(Serialize)]
struct CreateRoomRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    topic: Option<&'a str>,
    visibility: &'static str,
    preset: &'static str,
    is_direct: bool,
    invite: Vec<&'a str>,
    creation_content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    room_alias_name: Option<&'a str>,
}

impl<'a> From<&'a MatrixCreateRoom> for CreateRoomRequest<'a> {
    fn from(request: &'a MatrixCreateRoom) -> Self {
        let visibility = match request.visibility() {
            MatrixRoomVisibility::Private => "private",
            MatrixRoomVisibility::Public => "public",
        };
        let preset = match request.preset() {
            MatrixRoomPreset::PrivateChat => "private_chat",
            MatrixRoomPreset::PublicChat => "public_chat",
            MatrixRoomPreset::TrustedPrivateChat => "trusted_private_chat",
        };
        let creation_content = match request.kind() {
            MatrixRoomKind::Conversation => json!({}),
            MatrixRoomKind::Space => json!({ "type": "m.space" }),
        };
        Self {
            name: request.name(),
            topic: request.topic(),
            visibility,
            preset,
            is_direct: request.direct(),
            invite: request.invite().iter().map(MatrixUserId::as_str).collect(),
            creation_content,
            room_alias_name: request
                .alias_localpart()
                .map(MatrixRoomAliasLocalpart::as_str),
        }
    }
}

#[derive(Deserialize)]
struct CreateRoomResponse {
    room_id: String,
}

#[derive(Deserialize)]
struct ResolveRoomAliasResponse {
    room_id: String,
}

#[derive(Deserialize)]
struct StateEventResponse {
    event_id: String,
}

async fn expect_empty_success(
    response: reqwest::Response,
    operation: MatrixOperation,
) -> MatrixResult<()> {
    expect_success_body(response, operation).await.map(|_| ())
}

async fn expect_success_body(
    response: reqwest::Response,
    operation: MatrixOperation,
) -> MatrixResult<Vec<u8>> {
    let status = response.status();
    let body = read_limited_body(response, operation).await?;
    if status.is_success() {
        return Ok(body);
    }
    let error = decode_matrix_error(&body, operation)?;
    Err(map_matrix_error(operation, status, &error))
}

fn map_create_transport_error(operation: MatrixOperation, error: &reqwest::Error) -> MatrixFailure {
    if error.is_connect() {
        return MatrixFailure::new(operation, MatrixFailureKind::DependencyUnavailable);
    }
    MatrixFailure::new(operation, MatrixFailureKind::UnknownCommit)
}

fn endpoint_with_segments(
    base: &Url,
    segments: &[&str],
    operation: MatrixOperation,
) -> MatrixResult<Url> {
    let mut endpoint = base.clone();
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    let mut path = endpoint
        .path_segments_mut()
        .map_err(|()| MatrixFailure::new(operation, MatrixFailureKind::InvalidConfiguration))?;
    path.clear();
    path.extend(segments);
    drop(path);
    Ok(endpoint)
}

fn matrix_server_name(room_id: &str, operation: MatrixOperation) -> MatrixResult<&str> {
    room_id
        .strip_prefix('!')
        .and_then(|value| value.rsplit_once(':'))
        .map(|(_, server_name)| server_name)
        .filter(|server_name| !server_name.is_empty())
        .ok_or_else(|| invalid_response(operation))
}

const fn invalid_response(operation: MatrixOperation) -> MatrixFailure {
    MatrixFailure::new(operation, MatrixFailureKind::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use agent_room_application::ports::{
        MatrixCreateRoom, MatrixOperation, MatrixRoomAliasLocalpart, MatrixRoomId, MatrixRoomKind,
        MatrixRoomPreset, MatrixRoomVisibility, MatrixUserId, RoomMembershipGateway,
        RoomProvisioningGateway, SecretValue,
    };
    use agent_room_domain::rooms::MatrixRoomReference;
    use axum::{
        Json, Router,
        extract::{Path, Query},
        http::HeaderMap,
        response::IntoResponse,
        routing::{get, post, put},
    };
    use serde::Deserialize;
    use serde_json::{Value, json};
    use tokio::{net::TcpListener, task::JoinHandle};

    use super::super::{
        MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner,
    };

    #[tokio::test]
    async fn 受管_agent_通过身份断言加入和退出房间() {
        let server = TestServer::start().await;
        let provisioner = Arc::new(provisioner(&server.url));
        let user_id = MatrixUserId::new(
            "@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.agent-room.localhost",
        )
        .expect("用户标识有效");
        let membership = provisioner
            .room_membership(user_id.clone())
            .expect("受管用户可绑定成员能力");
        let room =
            MatrixRoomReference::new("!lobby:matrix.agent-room.localhost").expect("房间标识有效");

        membership.join(&room).await.expect("加入应成功");
        membership.leave(&room).await.expect("退出应成功");

        assert_eq!(server.calls().await, vec!["join", "leave"]);
        assert_eq!(
            server.asserted_users().await,
            vec![user_id.clone(), user_id]
        );
    }

    #[test]
    fn 普通_matrix_用户不能绑定_application_service_成员能力() {
        let provisioner = Arc::new(provisioner("http://127.0.0.1:9"));
        let ordinary =
            MatrixUserId::new("@ordinary:matrix.agent-room.localhost").expect("用户标识有效");

        let failure = provisioner
            .room_membership(ordinary)
            .expect_err("必须拒绝普通用户");

        assert_eq!(failure.operation(), MatrixOperation::Join);
    }

    #[tokio::test]
    async fn application_service_可创建解析并挂载大厅房间() {
        let server = TestServer::start().await;
        let provisioner = provisioner(&server.url);
        let alias = MatrixRoomAliasLocalpart::new("agent-room-space-general").expect("别名有效");
        let request = MatrixCreateRoom::new(
            Some("General".to_owned()),
            Some("Public lobby".to_owned()),
            MatrixRoomVisibility::Public,
            MatrixRoomPreset::PublicChat,
            false,
            Vec::new(),
        )
        .expect("建房请求有效")
        .with_kind(MatrixRoomKind::Space)
        .with_alias_localpart(alias.clone());

        let created = provisioner.create_room(&request).await.expect("建房成功");
        let resolved = provisioner
            .resolve_room_alias(&alias)
            .await
            .expect("别名解析成功");
        let child =
            MatrixRoomId::new("!child:matrix.agent-room.localhost").expect("子房间标识有效");
        let event = provisioner
            .attach_child(&created, &child)
            .await
            .expect("挂载成功");

        assert_eq!(created.as_str(), "!space:matrix.agent-room.localhost");
        assert_eq!(resolved, created);
        assert_eq!(event.as_str(), "$space-child-event");
        assert_eq!(server.calls().await, vec!["create", "resolve", "attach"]);
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

    struct TestServer {
        url: String,
        state: Arc<TestState>,
        task: JoinHandle<()>,
    }

    #[derive(Default)]
    struct TestState {
        calls: tokio::sync::Mutex<Vec<&'static str>>,
        asserted_users: tokio::sync::Mutex<Vec<MatrixUserId>>,
    }

    impl TestServer {
        async fn start() -> Self {
            let state = Arc::new(TestState::default());
            let app = Router::new()
                .route("/_matrix/client/v3/createRoom", post(create_room))
                .route(
                    "/_matrix/client/v3/directory/room/{alias}",
                    get(resolve_room),
                )
                .route(
                    "/_matrix/client/v3/rooms/{room}/state/m.space.child/{child}",
                    put(attach_child),
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

        async fn calls(&self) -> Vec<&'static str> {
            self.state.calls.lock().await.clone()
        }

        async fn asserted_users(&self) -> Vec<MatrixUserId> {
            self.state.asserted_users.lock().await.clone()
        }
    }

    impl Drop for TestServer {
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

    async fn create_room(
        axum::extract::State(state): axum::extract::State<Arc<TestState>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        assert_eq!(body["room_alias_name"], "agent-room-space-general");
        assert_eq!(body["creation_content"]["type"], "m.space");
        state.calls.lock().await.push("create");
        Json(json!({ "room_id": "!space:matrix.agent-room.localhost" }))
    }

    async fn resolve_room(
        axum::extract::State(state): axum::extract::State<Arc<TestState>>,
        Path(alias): Path<String>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        assert_eq!(
            alias,
            "#agent-room-space-general:matrix.agent-room.localhost"
        );
        state.calls.lock().await.push("resolve");
        Json(json!({ "room_id": "!space:matrix.agent-room.localhost" }))
    }

    async fn attach_child(
        axum::extract::State(state): axum::extract::State<Arc<TestState>>,
        Path((room, child)): Path<(String, String)>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        assert_eq!(room, "!space:matrix.agent-room.localhost");
        assert_eq!(child, "!child:matrix.agent-room.localhost");
        assert_eq!(body["via"], json!(["matrix.agent-room.localhost"]));
        state.calls.lock().await.push("attach");
        Json(json!({ "event_id": "$space-child-event" }))
    }

    #[derive(Deserialize)]
    struct UserQuery {
        user_id: String,
    }

    async fn change_membership(
        axum::extract::State(state): axum::extract::State<Arc<TestState>>,
        Path((_room, action)): Path<(String, String)>,
        Query(query): Query<UserQuery>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        assert_authentication(&headers);
        let action = match action.as_str() {
            "join" => "join",
            "leave" => "leave",
            unexpected => panic!("未知动作 {unexpected}"),
        };
        state.calls.lock().await.push(action);
        state
            .asserted_users
            .lock()
            .await
            .push(MatrixUserId::new(query.user_id).expect("断言用户有效"));
        Json(json!({}))
    }
}
