use std::{collections::BTreeMap, sync::Arc, time::Duration};

use agent_room_application::ports::{
    MatrixFailureKind, MatrixHumanSessionIssuer, MatrixUserId, SecretValue,
};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::Mutex};

use crate::{MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner};

const AS_TOKEN: &str = "as-token-for-tests";
const SERVER: &str = "matrix.agent-room.localhost";
const LOCALPART: &str = "user-0123456789abcdef0123456789abcdef";
const HUMAN: &str = "@user-0123456789abcdef0123456789abcdef:matrix.agent-room.localhost";

/// 假的 Synapse：记下收到了什么，按设定回答。
struct Synapse {
    /// 账户已经有了（以前走 SSO 建的）。
    exists: bool,
    displayname: Option<String>,
    /// 登录回答里的用户，不设就是请求的那个。
    login_user: Option<String>,
    calls: Vec<(String, Value)>,
}

type Shared = Arc<Mutex<Synapse>>;

fn synapse(exists: bool, displayname: Option<&str>) -> Shared {
    Arc::new(Mutex::new(Synapse {
        exists,
        displayname: displayname.map(str::to_owned),
        login_user: None,
        calls: Vec::new(),
    }))
}

#[tokio::test]
async fn 第一次用的人先注册_写上名字_再登录出带刷新令牌的设备() {
    let state = synapse(false, None);
    let url = start(state.clone()).await;

    let session = provisioner(&url)
        .issue_human_session(&user(HUMAN), "林雨")
        .await
        .expect("发出聊天登录");

    assert_eq!(session.metadata().user_id().as_str(), HUMAN);
    assert_eq!(session.metadata().device_id().as_str(), "DESKTOPDEV");
    assert_eq!(session.access_token().expose(), "syt_access");
    assert_eq!(
        session.refresh_token().map(SecretValue::expose),
        Some("syr_refresh")
    );
    let calls = state.lock().await.calls.clone();
    let paths: Vec<&str> = calls.iter().map(|(path, _)| path.as_str()).collect();
    assert_eq!(
        paths,
        ["register", "get-displayname", "put-displayname", "login"]
    );
    assert_eq!(
        calls[0].1,
        json!({ "type": "m.login.application_service", "username": LOCALPART, "inhibit_login": true })
    );
    assert_eq!(calls[2].1, json!({ "displayname": "林雨" }));
    assert_eq!(
        calls[3].1,
        json!({
            "type": "m.login.application_service",
            "identifier": { "type": "m.id.user", "user": HUMAN },
            "initial_device_display_name": "Agent Room Desktop",
            "refresh_token": true,
        })
    );
}

#[tokio::test]
async fn 已经有账户和名字的不改名字_直接登录() {
    let state = synapse(true, Some("Rainy"));
    let url = start(state.clone()).await;

    provisioner(&url)
        .issue_human_session(&user(HUMAN), "林雨")
        .await
        .expect("发出聊天登录");

    let paths: Vec<String> = state
        .lock()
        .await
        .calls
        .iter()
        .map(|(path, _)| path.clone())
        .collect();
    assert_eq!(paths, ["register", "get-displayname", "login"]);
}

#[tokio::test]
async fn 注册以后没写成名字的_下次补上() {
    // 应用服务注册的账户，Synapse 把显示名设成用户名。
    let state = synapse(true, Some(LOCALPART));
    let url = start(state.clone()).await;

    provisioner(&url)
        .issue_human_session(&user(HUMAN), "林雨")
        .await
        .expect("发出聊天登录");

    assert_eq!(state.lock().await.displayname.as_deref(), Some("林雨"));
}

#[tokio::test]
async fn 不替_agent_和别的服务器的人登录() {
    let state = synapse(false, None);
    let url = start(state.clone()).await;
    for other in [
        "@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.agent-room.localhost",
        "@user-0123456789abcdef0123456789abcdef:remote.example",
    ] {
        let failure = provisioner(&url)
            .issue_human_session(&user(other), "林雨")
            .await
            .expect_err("不能替这个用户登录");
        assert_eq!(failure.kind(), MatrixFailureKind::Forbidden);
    }
    assert!(state.lock().await.calls.is_empty());
}

#[tokio::test]
async fn 登录回来的不是这个人就不收() {
    let state = synapse(true, Some("Rainy"));
    state.lock().await.login_user = Some(format!("@someone-else:{SERVER}"));
    let url = start(state.clone()).await;

    let failure = provisioner(&url)
        .issue_human_session(&user(HUMAN), "林雨")
        .await
        .expect_err("回答对不上");
    assert_eq!(failure.kind(), MatrixFailureKind::InvalidResponse);
}

fn user(value: &str) -> MatrixUserId {
    MatrixUserId::new(value).expect("用户有效")
}

fn provisioner(url: &str) -> MatrixApplicationServiceProvisioner {
    MatrixApplicationServiceProvisioner::new(
        MatrixApplicationServiceConfiguration::new(
            url,
            SERVER.to_owned(),
            SecretValue::new(AS_TOKEN.to_owned()).expect("令牌有效"),
            Duration::from_secs(5),
        )
        .expect("配置有效"),
    )
    .expect("客户端有效")
}

fn assert_application_service(headers: &HeaderMap) {
    assert_eq!(
        headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some(format!("Bearer {AS_TOKEN}").as_str())
    );
}

async fn register(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert_application_service(&headers);
    let mut synapse = state.lock().await;
    synapse.calls.push(("register".to_owned(), body));
    if synapse.exists {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "errcode": "M_USER_IN_USE", "error": "taken" })),
        )
            .into_response();
    }
    synapse.exists = true;
    synapse.displayname = Some(LOCALPART.to_owned());
    Json(json!({ "user_id": HUMAN })).into_response()
}

async fn displayname(
    State(state): State<Shared>,
    Query(query): Query<BTreeMap<String, String>>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    assert_application_service(&headers);
    assert_eq!(
        query.get("user_id").map(String::as_str),
        Some(HUMAN),
        "冒充本人"
    );
    let mut synapse = state.lock().await;
    if let Some(Json(body)) = body {
        synapse.displayname = body["displayname"].as_str().map(str::to_owned);
        synapse.calls.push(("put-displayname".to_owned(), body));
        return Json(json!({})).into_response();
    }
    synapse
        .calls
        .push(("get-displayname".to_owned(), Value::Null));
    match &synapse.displayname {
        Some(name) => Json(json!({ "displayname": name })).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "errcode": "M_NOT_FOUND", "error": "none" })),
        )
            .into_response(),
    }
}

async fn login(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert_application_service(&headers);
    let mut synapse = state.lock().await;
    let user = synapse
        .login_user
        .clone()
        .unwrap_or_else(|| HUMAN.to_owned());
    synapse.calls.push(("login".to_owned(), body));
    Json(json!({
        "user_id": user,
        "device_id": "DESKTOPDEV",
        "access_token": "syt_access",
        "refresh_token": "syr_refresh",
        "expires_in_ms": 300_000,
    }))
    .into_response()
}

async fn start(state: Shared) -> String {
    let app = Router::new()
        .route("/_matrix/client/v3/register", post(register))
        .route(
            "/_matrix/client/v3/profile/{user}/displayname",
            get(displayname).put(displayname),
        )
        .route("/_matrix/client/v3/login", post(login))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("测试端口可用");
    let address = listener.local_addr().expect("测试地址有效");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("测试服务可运行");
    });
    format!("http://{address}")
}
