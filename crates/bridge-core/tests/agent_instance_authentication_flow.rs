use std::sync::Arc;

use agent_room_application::ports::{AgentInstanceVerificationRecord, DeviceSignature, PortFuture};
use agent_room_bridge_core::{
    agent_verification::{
        AgentInstanceMessageAuthenticator, AgentInstanceMessageAuthenticatorDependencies,
        AgentInstanceVerificationGateway, AgentInstanceVerificationGatewayFailure,
        AgentInstanceVerificationGatewayFailureKind, AgentInstanceVerificationGatewayResult,
    },
    messages::{
        MessageAuthenticationDecision, MessageAuthenticationFailureKind, MessageEventAuthenticator,
    },
};
use agent_room_domain::{
    agents::AgentInstancePublicSigningKey,
    ids::{AgentId, AgentInstanceId},
    time::UtcMillis,
};
use agent_room_identity_adapter::{Ed25519AgentInstanceSignatureVerifier, Ed25519DeviceSigningKey};
use uuid::Uuid;

const AGENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
const OTHER_AGENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
const INSTANCE_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";

struct 固定验签材料网关(
    AgentInstanceVerificationGatewayResult<AgentInstanceVerificationRecord>,
);

impl AgentInstanceVerificationGateway for 固定验签材料网关 {
    fn resolve(
        &self,
        _instance_id: AgentInstanceId,
    ) -> PortFuture<'_, AgentInstanceVerificationGatewayResult<AgentInstanceVerificationRecord>>
    {
        let result = self.0.clone();
        Box::pin(async move { result })
    }
}

#[tokio::test]
async fn 撤销前历史事件保留可信标记而撤销时刻及之后拒绝() {
    let signing_key = signing_key();
    let signature = signing_key.sign(b"signed-event").expect("测试事件可签名");
    let authenticator = authenticator(Ok(record(&signing_key, Some(time(3_000)))));

    let historical = authenticate(&authenticator, agent_id(), time(2_999), &signature)
        .await
        .expect("历史事件验签成功");
    let at_revocation = authenticate(&authenticator, agent_id(), time(3_000), &signature)
        .await
        .expect("撤销时刻事件得到业务判定");
    let before_registration = authenticate(&authenticator, agent_id(), time(999), &signature)
        .await
        .expect("注册前事件得到业务判定");

    assert_eq!(
        historical,
        MessageAuthenticationDecision::TrustedHistoricalRevoked
    );
    assert_eq!(
        at_revocation,
        MessageAuthenticationDecision::RevokedInstance
    );
    assert_eq!(
        before_registration,
        MessageAuthenticationDecision::OutsideInstanceValidityWindow
    );
}

#[tokio::test]
async fn 活跃实例必须同时满足归属和_ed25519_签名() {
    let signing_key = signing_key();
    let authenticator = authenticator(Ok(record(&signing_key, None)));
    let valid_signature = signing_key.sign(b"signed-event").expect("测试事件可签名");
    let forged_signature = signing_key.sign(b"other-event").expect("伪造样本可签名");

    assert_eq!(
        authenticate(&authenticator, agent_id(), time(2_000), &valid_signature)
            .await
            .expect("有效事件验签成功"),
        MessageAuthenticationDecision::Trusted
    );
    assert_eq!(
        authenticate(
            &authenticator,
            other_agent_id(),
            time(2_000),
            &valid_signature,
        )
        .await
        .expect("错误归属得到业务判定"),
        MessageAuthenticationDecision::AgentInstanceMismatch
    );
    assert_eq!(
        authenticate(&authenticator, agent_id(), time(2_000), &forged_signature)
            .await
            .expect("错误签名得到业务判定"),
        MessageAuthenticationDecision::InvalidSignature
    );
}

#[tokio::test]
async fn 未知实例被隔离而控制面不可用会中止整个同步批次() {
    let signature = signing_key().sign(b"signed-event").expect("测试事件可签名");
    let unknown = authenticator(Err(gateway_failure(
        AgentInstanceVerificationGatewayFailureKind::NotFound,
    )));
    let unavailable = authenticator(Err(gateway_failure(
        AgentInstanceVerificationGatewayFailureKind::Unavailable,
    )));

    assert_eq!(
        authenticate(&unknown, agent_id(), time(2_000), &signature)
            .await
            .expect("未知实例得到隔离判定"),
        MessageAuthenticationDecision::UnknownInstance
    );
    let failure = authenticate(&unavailable, agent_id(), time(2_000), &signature)
        .await
        .expect_err("控制面不可用不能推进游标");
    assert_eq!(
        failure.kind(),
        MessageAuthenticationFailureKind::Unavailable
    );
}

fn authenticator(
    result: AgentInstanceVerificationGatewayResult<AgentInstanceVerificationRecord>,
) -> AgentInstanceMessageAuthenticator {
    AgentInstanceMessageAuthenticator::new(AgentInstanceMessageAuthenticatorDependencies {
        verification: Arc::new(固定验签材料网关(result)),
        signatures: Arc::new(Ed25519AgentInstanceSignatureVerifier),
    })
}

async fn authenticate(
    authenticator: &AgentInstanceMessageAuthenticator,
    actor_agent_id: AgentId,
    origin_server_timestamp: UtcMillis,
    signature: &DeviceSignature,
) -> Result<
    MessageAuthenticationDecision,
    agent_room_bridge_core::messages::MessageAuthenticationFailure,
> {
    authenticator
        .authenticate(
            actor_agent_id,
            instance_id(),
            origin_server_timestamp,
            b"signed-event",
            signature,
        )
        .await
}

fn record(
    signing_key: &Ed25519DeviceSigningKey,
    invalidated_at: Option<UtcMillis>,
) -> AgentInstanceVerificationRecord {
    let public_key = signing_key.public_key().expect("测试公钥可导出");
    AgentInstanceVerificationRecord {
        instance_id: instance_id(),
        agent_id: agent_id(),
        public_signing_key: AgentInstancePublicSigningKey::new(public_key.as_bytes().to_vec())
            .expect("实例公钥有效"),
        registered_at: time(1_000),
        invalidated_at,
    }
}

fn signing_key() -> Ed25519DeviceSigningKey {
    Ed25519DeviceSigningKey::generate().expect("测试签名密钥可生成")
}

fn gateway_failure(
    kind: AgentInstanceVerificationGatewayFailureKind,
) -> AgentInstanceVerificationGatewayFailure {
    AgentInstanceVerificationGatewayFailure::new(kind)
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(uuid(AGENT_ID))
}

fn other_agent_id() -> AgentId {
    AgentId::from_uuid(uuid(OTHER_AGENT_ID))
}

fn instance_id() -> AgentInstanceId {
    AgentInstanceId::from_uuid(uuid(INSTANCE_ID))
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).expect("测试 UUID 有效")
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
