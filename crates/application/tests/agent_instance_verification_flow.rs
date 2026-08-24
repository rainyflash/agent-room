use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use agent_room_application::{
    agent_instance_verification::{
        AgentInstanceVerificationDependencies, AgentInstanceVerificationFailureKind,
        AgentInstanceVerificationService, AgentInstanceVerificationUseCases,
        ResolveAgentInstanceVerification,
    },
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentInstanceVerificationRecord, AgentInstanceVerificationRepository, Clock, PortFuture,
        PrincipalAccount,
    },
};
use agent_room_domain::{
    agents::AgentInstancePublicSigningKey,
    identity::Principal,
    ids::{AgentId, AgentInstanceId, DeviceId, PrincipalId},
    time::UtcMillis,
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

struct 固定时钟;

impl Clock for 固定时钟 {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

struct 记录仓库 {
    calls: AtomicUsize,
    outcome: Mutex<RepositoryResult<Option<AgentInstanceVerificationRecord>>>,
}

impl AgentInstanceVerificationRepository for 记录仓库 {
    fn find_verification_record(
        &self,
        _instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentInstanceVerificationRecord>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let result = self.outcome.lock().expect("仓库结果锁可用").clone();
        Box::pin(async move { result })
    }
}

#[tokio::test]
async fn 已认证设备可以读取实例的历史验签有效期() {
    let record = verification_record(Some(NOW - 1_000));
    let repository = Arc::new(记录仓库 {
        calls: AtomicUsize::new(0),
        outcome: Mutex::new(Ok(Some(record.clone()))),
    });
    let service = verification_service(repository.clone());

    let resolved = service
        .resolve(request(record.instance_id, NOW + 60_000))
        .await
        .expect("有效查询成功");

    assert_eq!(resolved, record);
    assert_eq!(repository.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn 过期设备会话在访问仓库前被拒绝() {
    let record = verification_record(None);
    let repository = Arc::new(记录仓库 {
        calls: AtomicUsize::new(0),
        outcome: Mutex::new(Ok(Some(record.clone()))),
    });
    let service = verification_service(repository.clone());

    let failure = service
        .resolve(request(record.instance_id, NOW))
        .await
        .expect_err("过期设备必须拒绝");

    assert_eq!(
        failure.kind(),
        AgentInstanceVerificationFailureKind::Unauthorized
    );
    assert_eq!(repository.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn 仓库损坏或失效时间逆序不会泄漏成可信验签材料() {
    let mut incoherent = verification_record(None);
    incoherent.invalidated_at = Some(time(NOW - 20_000));
    let repository = Arc::new(记录仓库 {
        calls: AtomicUsize::new(0),
        outcome: Mutex::new(Ok(Some(incoherent.clone()))),
    });
    let service = verification_service(repository);
    let failure = service
        .resolve(request(incoherent.instance_id, NOW + 60_000))
        .await
        .expect_err("时间逆序必须拒绝");
    assert_eq!(
        failure.kind(),
        AgentInstanceVerificationFailureKind::Internal
    );

    let unavailable = Arc::new(记录仓库 {
        calls: AtomicUsize::new(0),
        outcome: Mutex::new(Err(RepositoryError::new(
            "agent_instance.verification.find",
            RepositoryErrorKind::Unavailable,
        ))),
    });
    let failure = verification_service(unavailable)
        .resolve(request(incoherent.instance_id, NOW + 60_000))
        .await
        .expect_err("依赖故障必须显式上浮");
    assert_eq!(
        failure.kind(),
        AgentInstanceVerificationFailureKind::DependencyUnavailable
    );
}

fn verification_service(repository: Arc<记录仓库>) -> AgentInstanceVerificationService {
    AgentInstanceVerificationService::new(AgentInstanceVerificationDependencies {
        records: repository,
        clock: Arc::new(固定时钟),
    })
}

fn request(
    instance_id: AgentInstanceId,
    access_token_expires_at: i64,
) -> ResolveAgentInstanceVerification {
    let principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    ResolveAgentInstanceVerification {
        actor: AuthenticatedDevice {
            account: PrincipalAccount {
                principal: Principal::new(principal_id),
                matrix_user_id: "@verification-client:matrix.test".to_owned(),
                display_name: "验签客户端".to_owned(),
                avatar_content_id: None,
                locale: "zh-CN".to_owned(),
            },
            device_id: DeviceId::from_uuid(Uuid::now_v7()),
            access_token_expires_at: time(access_token_expires_at),
        },
        instance_id,
    }
}

fn verification_record(invalidated_at: Option<i64>) -> AgentInstanceVerificationRecord {
    AgentInstanceVerificationRecord {
        instance_id: AgentInstanceId::from_uuid(Uuid::now_v7()),
        agent_id: AgentId::from_uuid(Uuid::now_v7()),
        public_signing_key: AgentInstancePublicSigningKey::new(vec![7; 32]).expect("实例公钥有效"),
        registered_at: time(NOW - 10_000),
        invalidated_at: invalidated_at.map(time),
    }
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
