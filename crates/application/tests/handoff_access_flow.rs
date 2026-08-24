use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use agent_room_application::{
    devices::AuthenticatedDevice,
    handoffs::{
        AuthorizeHandoff, HandoffAccessDependencies, HandoffAccessFailureKind,
        HandoffAccessService, HandoffAccessUseCases, HandoffAuthorizationDecision,
        ResolveHandoffInstance,
    },
    persistence::RepositoryResult,
    ports::{
        Clock, HandoffAccessRepository, HandoffAuthorizationSnapshot, HandoffInstanceAccessRecord,
        PortFuture, PrincipalAccount,
    },
};
use agent_room_domain::{
    agents::AgentRole,
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

struct 访问仓库 {
    authorization_calls: AtomicUsize,
    directory_calls: AtomicUsize,
    authorization: Mutex<RepositoryResult<Option<HandoffAuthorizationSnapshot>>>,
    directory: Mutex<RepositoryResult<Option<HandoffInstanceAccessRecord>>>,
}

impl HandoffAccessRepository for 访问仓库 {
    fn inspect_authorization(
        &self,
        _principal_id: PrincipalId,
        _requester_instance_id: AgentInstanceId,
        _target_instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<HandoffAuthorizationSnapshot>>> {
        self.authorization_calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self.authorization.lock().expect("授权结果锁可用").clone();
        Box::pin(async move { outcome })
    }

    fn find_instance_access(
        &self,
        _principal_id: PrincipalId,
        _instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<HandoffInstanceAccessRecord>>> {
        self.directory_calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self.directory.lock().expect("目录结果锁可用").clone();
        Box::pin(async move { outcome })
    }
}

#[tokio::test]
async fn 同一主体可把上下文从有操作权的请求实例交给有操作权的目标实例() {
    let fixture = Fixture::new();
    let repository = fixture.repository();
    let service = service(repository.clone());

    let decision = service
        .authorize(fixture.authorization_request(NOW + 60_000))
        .await
        .expect("授权查询成功");

    assert_eq!(decision, HandoffAuthorizationDecision::Allowed);
    assert_eq!(repository.authorization_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn 主体声明不匹配或任一实例只有查看权限时拒绝交接() {
    let fixture = Fixture::new();
    let repository = fixture.repository();
    let service = service(repository.clone());
    let mut wrong_principal = fixture.authorization_request(NOW + 60_000);
    wrong_principal.principal_id = PrincipalId::from_uuid(Uuid::now_v7());
    assert_eq!(
        service
            .authorize(wrong_principal)
            .await
            .expect("主体不匹配是业务拒绝"),
        HandoffAuthorizationDecision::Denied
    );
    assert_eq!(repository.authorization_calls.load(Ordering::SeqCst), 0);

    let mut snapshot = fixture.snapshot();
    snapshot.target.role = Some(AgentRole::Viewer);
    *repository.authorization.lock().expect("授权结果锁可用") = Ok(Some(snapshot));
    assert_eq!(
        service
            .authorize(fixture.authorization_request(NOW + 60_000))
            .await
            .expect("权限不足是业务拒绝"),
        HandoffAuthorizationDecision::Denied
    );
}

#[tokio::test]
async fn 实例目录只返回主体可操作且仍有效的精确设备地址() {
    let fixture = Fixture::new();
    let repository = fixture.repository();
    let service = service(repository.clone());

    let resolved = service
        .resolve_instance(ResolveHandoffInstance {
            actor: fixture.actor(NOW + 60_000),
            instance_id: fixture.target_instance,
        })
        .await
        .expect("目标实例可解析");

    assert_eq!(resolved.agent_id, fixture.target_agent);
    assert_eq!(resolved.instance_id, fixture.target_instance);
    assert_eq!(resolved.matrix_user_id, "@target:matrix.test");
    assert_eq!(resolved.matrix_device_id, "TARGET_DEVICE");
    assert_eq!(repository.directory_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn 过期设备在查询授权事实前失败且无权实例按不存在处理() {
    let fixture = Fixture::new();
    let repository = fixture.repository();
    let service = service(repository.clone());
    let failure = service
        .authorize(fixture.authorization_request(NOW))
        .await
        .expect_err("过期设备必须失败");
    assert_eq!(failure.kind(), HandoffAccessFailureKind::Unauthorized);
    assert_eq!(repository.authorization_calls.load(Ordering::SeqCst), 0);

    let mut inaccessible = fixture.target_record();
    inaccessible.active = false;
    *repository.directory.lock().expect("目录结果锁可用") = Ok(Some(inaccessible));
    let failure = service
        .resolve_instance(ResolveHandoffInstance {
            actor: fixture.actor(NOW + 60_000),
            instance_id: fixture.target_instance,
        })
        .await
        .expect_err("失效实例不得泄漏目录地址");
    assert_eq!(failure.kind(), HandoffAccessFailureKind::NotFound);
}

struct Fixture {
    principal: PrincipalId,
    requester_agent: AgentId,
    requester_instance: AgentInstanceId,
    target_agent: AgentId,
    target_instance: AgentInstanceId,
    device: DeviceId,
}

impl Fixture {
    fn new() -> Self {
        Self {
            principal: PrincipalId::from_uuid(Uuid::now_v7()),
            requester_agent: AgentId::from_uuid(Uuid::now_v7()),
            requester_instance: AgentInstanceId::from_uuid(Uuid::now_v7()),
            target_agent: AgentId::from_uuid(Uuid::now_v7()),
            target_instance: AgentInstanceId::from_uuid(Uuid::now_v7()),
            device: DeviceId::from_uuid(Uuid::now_v7()),
        }
    }

    fn repository(&self) -> Arc<访问仓库> {
        Arc::new(访问仓库 {
            authorization_calls: AtomicUsize::new(0),
            directory_calls: AtomicUsize::new(0),
            authorization: Mutex::new(Ok(Some(self.snapshot()))),
            directory: Mutex::new(Ok(Some(self.target_record()))),
        })
    }

    fn snapshot(&self) -> HandoffAuthorizationSnapshot {
        HandoffAuthorizationSnapshot {
            requester: HandoffInstanceAccessRecord {
                instance_id: self.requester_instance,
                agent_id: self.requester_agent,
                device_id: self.device,
                matrix_user_id: "@requester:matrix.test".to_owned(),
                matrix_device_id: "REQUESTER_DEVICE".to_owned(),
                role: Some(AgentRole::Owner),
                active: true,
            },
            target: self.target_record(),
        }
    }

    fn target_record(&self) -> HandoffInstanceAccessRecord {
        HandoffInstanceAccessRecord {
            instance_id: self.target_instance,
            agent_id: self.target_agent,
            device_id: DeviceId::from_uuid(Uuid::now_v7()),
            matrix_user_id: "@target:matrix.test".to_owned(),
            matrix_device_id: "TARGET_DEVICE".to_owned(),
            role: Some(AgentRole::Operator),
            active: true,
        }
    }

    fn actor(&self, expires_at: i64) -> AuthenticatedDevice {
        AuthenticatedDevice {
            account: PrincipalAccount {
                principal: Principal::new(self.principal),
                matrix_user_id: "@owner:matrix.test".to_owned(),
                display_name: "交接授权主体".to_owned(),
                avatar_content_id: None,
                locale: "zh-CN".to_owned(),
            },
            device_id: self.device,
            access_token_expires_at: time(expires_at),
        }
    }

    fn authorization_request(&self, expires_at: i64) -> AuthorizeHandoff {
        AuthorizeHandoff {
            actor: self.actor(expires_at),
            principal_id: self.principal,
            requester_agent_id: self.requester_agent,
            requester_instance_id: self.requester_instance,
            target_agent_id: self.target_agent,
            target_instance_id: self.target_instance,
        }
    }
}

fn service(repository: Arc<访问仓库>) -> HandoffAccessService {
    HandoffAccessService::new(HandoffAccessDependencies {
        access: repository,
        clock: Arc::new(固定时钟),
    })
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
