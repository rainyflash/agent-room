//! 本机 Bridge 替这台电脑的账户认领（3P-4a）：设备签名对着方法、路径和正文，认出来是这台电脑的账户；
//! 不带签名、签名对不上的都不认。

use agent_room_application::{
    devices::{
        AuthenticateDeviceRequest, AuthenticatedDevice, DeviceAuthorizationResult,
        DeviceAuthorizationUseCases, DeviceCredentials, RefreshDeviceSession, RegisterDevice,
        RevokedDevice,
    },
    ports::{PrincipalAccount, SecretFactory},
};
use agent_room_domain::{devices::Device, identity::Principal, ids::DeviceId};
use agent_room_identity_adapter::SecureSecretFactory;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

use super::*;

const DEVICE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e4a";
const CLAIMS: &str = "/devices/current/project-rooms/claims";

/// 只认一次设备签名：方法、路径和正文都要对得上。
#[derive(Default)]
pub(super) struct FakeDevices {
    expected: Mutex<Option<(String, String)>>,
}

impl FakeDevices {
    fn expecting(target: &str, body: &str) -> Arc<Self> {
        Arc::new(Self {
            expected: Mutex::new(Some((target.to_owned(), body.to_owned()))),
        })
    }
}

impl DeviceAuthorizationUseCases for FakeDevices {
    fn register_device(
        &self,
        _request: RegisterDevice,
    ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>> {
        Box::pin(async { unreachable!("认领不注册设备") })
    }

    fn authenticate_device<'a>(
        &'a self,
        request: AuthenticateDeviceRequest<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<AuthenticatedDevice>> {
        let (target, body) = self
            .expected
            .lock()
            .unwrap()
            .take()
            .expect("登记了设备签名");
        assert_eq!(request.access_token.expose(), "device-access-token");
        assert_eq!(request.proof.method(), "POST");
        assert_eq!(request.proof.request_target(), target, "签名覆盖的路径");
        assert_eq!(
            request.proof.body_digest(),
            &SecureSecretFactory.digest(&body),
            "签名覆盖的正文"
        );
        Box::pin(async move {
            Ok(AuthenticatedDevice {
                account: PrincipalAccount {
                    principal: Principal::new(PrincipalId::from_uuid(
                        Uuid::parse_str(PRINCIPAL_UUID).unwrap(),
                    )),
                    matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
                    display_name: "Agent Room User".to_owned(),
                    avatar_content_id: None,
                    locale: "zh-CN".to_owned(),
                },
                device_id: DeviceId::from_uuid(Uuid::parse_str(DEVICE_UUID).unwrap()),
                access_token_expires_at: time(1_700_000_900_000),
            })
        })
    }

    fn refresh_device_session<'a>(
        &'a self,
        _request: RefreshDeviceSession<'a>,
    ) -> PortFuture<'a, DeviceAuthorizationResult<DeviceCredentials>> {
        Box::pin(async { unreachable!("认领不刷新设备会话") })
    }

    fn list_devices(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<Vec<Device>>> {
        Box::pin(async { unreachable!("认领不列设备") })
    }

    fn revoke_device(
        &self,
        _principal_id: PrincipalId,
        _device_id: DeviceId,
    ) -> PortFuture<'_, DeviceAuthorizationResult<RevokedDevice>> {
        Box::pin(async { unreachable!("认领不撤销设备") })
    }
}

/// 本机 Bridge 发的请求：设备令牌和签名头，没有会话和来源。
fn device_request(path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, "Bearer device-access-token")
        .header("x-agent-room-device-id", DEVICE_UUID)
        .header("x-agent-room-proof-issued-at", "1700000000000")
        .header("x-agent-room-proof-nonce", "nonce-0123456789abcdef")
        .header(
            "x-agent-room-proof-signature",
            URL_SAFE_NO_PAD.encode([9_u8; 64]),
        )
        .body(Body::from(body.to_owned()))
        .unwrap()
}

fn device_actor_matches(actor: &PrivateRoomActor) -> bool {
    matches!(actor, PrivateRoomActor::Device(device)
        if device.account.principal.id().as_uuid().to_string() == PRINCIPAL_UUID
            && device.device_id.as_uuid().to_string() == DEVICE_UUID)
}

#[tokio::test]
async fn 本机设备签名认领_回答和网页的一样() {
    let rooms = Arc::new(FakeProjectRooms::default());
    let body =
        json!({ "repository": "octo-org/hello.world", "catalogId": CATALOG_UUID }).to_string();
    let response = router_with_devices(rooms.clone(), FakeDevices::expecting(CLAIMS, &body))
        .oneshot(device_request(CLAIMS, &body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let answer = response_json(response).await;
    assert_eq!(answer["claimId"], CLAIM_UUID);
    assert_eq!(answer["shareUrl"], "https://agent-room.test/r/aB3xYz");
    assert_eq!(
        answer["badgeMarkdown"],
        "[![Agent Room](https://agent-room.test/gh/octo-org/hello.world/badge.svg)](https://agent-room.test/r/aB3xYz)"
    );
    let claims = rooms.claims.lock().unwrap();
    assert!(
        device_actor_matches(&claims[0].actor),
        "认的是这台电脑的账户"
    );
    assert_eq!(claims[0].repository, "octo-org/hello.world");
    assert_eq!(claims[0].target, Some(catalog()));
}

#[tokio::test]
async fn 本机设备签名核对_没有正文签空串() {
    let rooms = Arc::new(FakeProjectRooms::default());
    *rooms.bound.lock().unwrap() = true;
    let target = format!("{CLAIMS}/{CLAIM_UUID}/check");
    let response = router_with_devices(rooms.clone(), FakeDevices::expecting(&target, ""))
        .oneshot(device_request(&target, ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["status"], "bound");
    let inspected = rooms.inspected.lock().unwrap();
    assert_eq!(inspected[0].0, "check");
    assert!(device_actor_matches(&inspected[0].1.actor));
    assert_eq!(inspected[0].1.claim_id, claim());
}

#[tokio::test]
async fn 没带设备令牌和签名的_不认() {
    let rooms = Arc::new(FakeProjectRooms::default());
    let body = json!({ "repository": "octo/hello" }).to_string();
    let unsigned = router_with_devices(rooms.clone(), Arc::default())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(CLAIMS)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsigned.status(), StatusCode::UNAUTHORIZED);
    assert!(rooms.claims.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 本机设备认领_用例不让时照样按种类回答() {
    let rooms = FakeProjectRooms::failing(ProjectRoomFailureKind::Forbidden);
    let body = json!({ "repository": "octo/hello" }).to_string();
    let response = router_with_devices(rooms, FakeDevices::expecting(CLAIMS, &body))
        .oneshot(device_request(CLAIMS, &body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await["code"],
        "project_room.forbidden"
    );
}
