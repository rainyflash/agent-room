use std::{collections::BTreeMap, sync::Arc, time::Duration};

use agent_room_application::ports::{
    MatrixDeviceId, MatrixFailureKind, MatrixHumanDeviceSignOut, MatrixUserId, SecretValue,
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
const HUMAN: &str = "@user-0123456789abcdef0123456789abcdef:matrix.agent-room.localhost";

/// 假的 Synapse：这个人有哪几台设备，记下删了哪些。
struct Synapse {
    devices: Vec<String>,
    deleted: Vec<Vec<String>>,
    unavailable: bool,
}

type Shared = Arc<Mutex<Synapse>>;

fn synapse(devices: &[&str]) -> Shared {
    Arc::new(Mutex::new(Synapse {
        devices: devices.iter().map(|device| (*device).to_owned()).collect(),
        deleted: Vec::new(),
        unavailable: false,
    }))
}

#[tokio::test]
async fn 留着这一台_别的设备一次注销() {
    let state = synapse(&["THISBROWSER", "OLDLAPTOP", "DESKTOPDEV"]);
    let url = start(state.clone()).await;

    let signed_out = provisioner(&url)
        .sign_out_other_devices(&user(HUMAN), Some(&device("THISBROWSER")))
        .await
        .expect("注销成功");

    assert_eq!(signed_out, 2);
    let synapse = state.lock().await;
    assert_eq!(synapse.deleted, [vec!["OLDLAPTOP", "DESKTOPDEV"]]);
    assert_eq!(synapse.devices, ["THISBROWSER"]);
}

#[tokio::test]
async fn 不留的时候全部注销() {
    let state = synapse(&["OLDLAPTOP", "DESKTOPDEV"]);
    let url = start(state.clone()).await;

    let signed_out = provisioner(&url)
        .sign_out_other_devices(&user(HUMAN), None)
        .await
        .expect("注销成功");

    assert_eq!(signed_out, 2);
    assert!(state.lock().await.devices.is_empty());
}

#[tokio::test]
async fn 只剩这一台时不发注销请求() {
    let state = synapse(&["THISBROWSER"]);
    let url = start(state.clone()).await;

    let signed_out = provisioner(&url)
        .sign_out_other_devices(&user(HUMAN), Some(&device("THISBROWSER")))
        .await
        .expect("没什么可注销的");

    assert_eq!(signed_out, 0);
    assert!(state.lock().await.deleted.is_empty());
}

#[tokio::test]
async fn 不碰_agent_和别的服务器的人() {
    let state = synapse(&["OLDLAPTOP"]);
    let url = start(state.clone()).await;
    for other in [
        "@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.agent-room.localhost",
        "@user-0123456789abcdef0123456789abcdef:remote.example",
    ] {
        let failure = provisioner(&url)
            .sign_out_other_devices(&user(other), None)
            .await
            .expect_err("不能替这个用户注销");
        assert_eq!(failure.kind(), MatrixFailureKind::Forbidden);
    }
    let synapse = state.lock().await;
    assert!(synapse.deleted.is_empty());
    assert_eq!(synapse.devices, ["OLDLAPTOP"]);
}

#[tokio::test]
async fn 聊天服务器出错时报出来() {
    let state = synapse(&["OLDLAPTOP"]);
    state.lock().await.unavailable = true;
    let url = start(state.clone()).await;

    let failure = provisioner(&url)
        .sign_out_other_devices(&user(HUMAN), None)
        .await
        .expect_err("聊天服务器出错");

    assert_eq!(failure.kind(), MatrixFailureKind::DependencyUnavailable);
}

fn user(value: &str) -> MatrixUserId {
    MatrixUserId::new(value).expect("用户有效")
}

fn device(value: &str) -> MatrixDeviceId {
    MatrixDeviceId::new(value).expect("设备号有效")
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

/// 以应用服务的身份、冒充本人。
fn assert_as_human(headers: &HeaderMap, query: &BTreeMap<String, String>) {
    assert_eq!(
        headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some(format!("Bearer {AS_TOKEN}").as_str())
    );
    assert_eq!(
        query.get("user_id").map(String::as_str),
        Some(HUMAN),
        "冒充本人"
    );
}

async fn list(
    State(state): State<Shared>,
    Query(query): Query<BTreeMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    assert_as_human(&headers, &query);
    let synapse = state.lock().await;
    if synapse.unavailable {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "errcode": "M_UNKNOWN", "error": "upstream" })),
        )
            .into_response();
    }
    let devices: Vec<Value> = synapse
        .devices
        .iter()
        .map(|device| json!({ "device_id": device, "display_name": null, "last_seen_ts": 1 }))
        .collect();
    Json(json!({ "devices": devices })).into_response()
}

async fn delete(
    State(state): State<Shared>,
    Query(query): Query<BTreeMap<String, String>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert_as_human(&headers, &query);
    assert!(body.get("auth").is_none(), "应用服务不带交互认证");
    let deleted: Vec<String> = body["devices"]
        .as_array()
        .expect("要注销的设备")
        .iter()
        .map(|device| device.as_str().expect("设备号").to_owned())
        .collect();
    let mut synapse = state.lock().await;
    synapse.devices.retain(|device| !deleted.contains(device));
    synapse.deleted.push(deleted);
    Json(json!({})).into_response()
}

async fn start(state: Shared) -> String {
    let app = Router::new()
        .route("/_matrix/client/v3/devices", get(list))
        .route("/_matrix/client/v3/delete_devices", post(delete))
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
