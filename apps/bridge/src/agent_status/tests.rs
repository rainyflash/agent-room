//! 写名片、报 Matrix 在线状态（`specs/agent-liveness/design.md` 第 3 步）。

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_room_application::ports::{
    Clock, DeviceSignature, MatrixAcceptedEvent, MatrixBackfillPage, MatrixBackfillRequest,
    MatrixCreateRoom, MatrixDeviceId, MatrixEvent, MatrixEventId, MatrixEventType, MatrixFailure,
    MatrixFailureKind, MatrixGateway, MatrixOperation, MatrixReceipt, MatrixResult,
    MatrixRoomAliasLocalpart, MatrixRoomId, MatrixSessionMetadata, MatrixStateEvent,
    MatrixStateKey, MatrixSyncBatch, MatrixSyncRequest, MatrixUserId, MatrixUserPresence,
    PortFuture,
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    ports::{
        AgentStatusStatePublisher, BridgeCredentialResult, DeviceSigningIdentity,
        StatusEventIdentifierFactory,
    },
    presence::AGENT_STATUS_EVENT_TYPE,
    status::{
        AgentStatusLeasePolicy, AgentStatusPublicationDependencies, AgentStatusPublicationService,
        AgentStatusRoomTarget, HostAgentState,
    },
};
use agent_room_domain::{
    agent_lifecycle::MatrixPresenceState::{self, Offline, Online, Unavailable},
    agent_status::AgentStatusVisibility,
    devices::DevicePublicSigningKey,
    ids::{AgentId, AgentInstanceId},
    time::{DurationMillis, UtcMillis},
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::AgentStatusPublicationHandle;

#[tokio::test]
async fn 确认服务器开着在线状态以后才写名片_只写一次() {
    let matrix = 在线状态网关::new(true);
    let publisher = Arc::new(记录状态发布器::default());
    let status = 名片句柄(&matrix, &publisher, "Codex Agent");

    // 第一次报就被限速：不知道开没开，先不写。
    matrix.fail_next_report(MatrixFailureKind::RateLimited);
    status.renew().await.expect("第一次同步之后");
    assert!(publisher.contents().is_empty(), "还没确认就不写名片");

    for _ in 0..3 {
        status.renew().await.expect("之后的同步");
    }
    let contents = publisher.contents();
    assert_eq!(contents.len(), 1, "名片只写一次");
    assert_eq!(contents[0]["liveness"], "presence");
    assert_eq!(
        matrix.reported(),
        [Unavailable],
        "确认时报此刻该报的：还没在等"
    );
}

#[tokio::test]
async fn 房间里已经有一样的名片就不写_改了名或者还是租约就写() {
    for (existing, writes) in [
        (Some(房间里的("Codex Agent", true)), 0),
        (Some(房间里的("Old Name", true)), 1),
        (Some(房间里的("Codex Agent", false)), 1),
        (None, 1),
    ] {
        let matrix = 在线状态网关::new(true);
        *matrix.room_state.lock().expect("锁可用") = existing;
        let publisher = Arc::new(记录状态发布器::default());
        let status = 名片句柄(&matrix, &publisher, "Codex Agent");
        status.renew().await.expect("同步之后");
        assert_eq!(publisher.contents().len(), writes);
        status.renew().await.expect("下一次同步");
        assert_eq!(publisher.contents().len(), writes, "这次连上以后只看一次");
        assert_eq!(*matrix.state_reads.lock().expect("锁可用"), 1);
    }
}

#[tokio::test]
async fn 读不出房间里的那条就这次先不写_下次同步再看() {
    let matrix = 在线状态网关::new(true);
    let publisher = Arc::new(记录状态发布器::default());
    let status = 名片句柄(&matrix, &publisher, "Codex Agent");

    matrix
        .state_failures
        .lock()
        .expect("锁可用")
        .push(MatrixFailureKind::Timeout);
    status.renew().await.expect("读不出来不算同步失败");
    assert!(publisher.contents().is_empty(), "不知道有没有就先不写");

    *matrix.room_state.lock().expect("锁可用") = Some(房间里的("Codex Agent", true));
    status.renew().await.expect("下一次同步");
    assert!(publisher.contents().is_empty(), "房间里已经有了");

    // 重连以后是新的句柄：照样先问服务器，不重写。
    let reconnected = 名片句柄(&matrix, &publisher, "Codex Agent");
    reconnected.renew().await.expect("重连以后的第一次同步");
    assert!(publisher.contents().is_empty());
}

#[tokio::test(start_paused = true)]
async fn 同步按在不在等报在线或离开_等完一分钟以后才算离开() {
    let matrix = 在线状态网关::new(true);
    let publisher = Arc::new(记录状态发布器::default());
    let status = 名片句柄(&matrix, &publisher, "Codex Agent");
    assert_eq!(status.sync_presence().await, Unavailable, "还没在等");

    // 私人房间里等着也算：在线状态按 Agent 算，不按房间算。
    status
        .note_inbox_wait(&私人房间(), 固定时钟.now(), true)
        .await
        .expect("开始等");
    assert_eq!(
        matrix.reported(),
        [Online],
        "开始等就马上报在线，不等下一次同步"
    );
    tokio::time::sleep(Duration::from_secs(4)).await;
    status
        .note_inbox_wait(&大厅(), 固定时钟.now(), true)
        .await
        .expect("挂着等的时候再报一次在等");
    status
        .note_inbox_wait(&大厅(), 固定时钟.now(), false)
        .await
        .expect("读到消息就不在等了");
    assert_eq!(matrix.reported(), [Online], "一直在等不重复报");
    assert_eq!(status.sync_presence().await, Online);

    tokio::time::sleep(Duration::from_secs(59)).await;
    assert_eq!(
        status.sync_presence().await,
        Online,
        "等完不到一分钟还算在等"
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        status.sync_presence().await,
        Unavailable,
        "等完一分钟以后报离开"
    );

    status
        .note_inbox_wait(&大厅(), 固定时钟.now(), true)
        .await
        .expect("又来等");
    assert_eq!(
        matrix.reported(),
        [Online, Online],
        "同步报过离开，再来等就再报在线"
    );
    assert!(publisher.contents().is_empty(), "等消息不写状态事件");
}

#[tokio::test]
async fn 服务器没开在线状态就改回写租约() {
    // 一种是读回自己一律离线（Synapse 关了在线状态），看两次才算；一种是根本不接报的接口，
    // 一次就算。
    for matrix in [在线状态网关::new(false), 在线状态网关::new(true)] {
        let publisher = Arc::new(记录状态发布器::default());
        let status = 名片句柄(&matrix, &publisher, "Codex Agent");
        if matrix.enabled {
            matrix.fail_next_report(MatrixFailureKind::NotFound);
        } else {
            status.renew().await.expect("第一次同步之后");
            assert!(publisher.contents().is_empty(), "只读回一次离线，先不写");
        }
        status.renew().await.expect("同步之后");
        let contents = publisher.contents();
        assert_eq!(contents.len(), 1);
        assert!(contents[0].get("liveness").is_none(), "改回写租约");
        assert!(contents[0]["leaseExpiresAt"].is_string());
        assert_eq!(
            status.sync_presence().await,
            Online,
            "写租约时同步照旧报在线"
        );

        status
            .note_inbox_wait(&大厅(), 固定时钟.now(), true)
            .await
            .expect("开始等");
        status.disconnect().await.expect("正常退出");
        let contents = publisher.contents();
        assert_eq!(contents.len(), 3, "写租约时开始等、退出各发一条");
        assert_eq!(contents[2]["status"], "offline");
    }
}

#[tokio::test]
async fn 正常退出报离线_旧版客户端报的工作状态照收不发() {
    let matrix = 在线状态网关::new(true);
    let publisher = Arc::new(记录状态发布器::default());
    let status = 名片句柄(&matrix, &publisher, "Codex Agent");
    status.renew().await.expect("同步之后写名片");

    status.acknowledge().await.expect("旧版客户端报离线");
    status.acknowledge().await.expect("旧版客户端报在忙");
    assert_eq!(publisher.contents().len(), 1, "只有名片");
    assert_eq!(matrix.reported(), [Unavailable], "报的工作状态不算退出");

    status.disconnect().await.expect("正常退出");
    assert_eq!(matrix.reported(), [Unavailable, Offline]);
    assert_eq!(publisher.contents().len(), 1);
}

fn 名片句柄(
    matrix: &Arc<在线状态网关>,
    publisher: &Arc<记录状态发布器>,
    name: &str,
) -> Arc<AgentStatusPublicationHandle> {
    Arc::new(AgentStatusPublicationHandle::with_presence(
        AgentStatusPublicationService::new(
            AgentStatusPublicationDependencies {
                identity: 身份(name),
                signer: Arc::new(测试签名身份),
                publisher: publisher.clone(),
                identifiers: Arc::new(版本七状态标识),
                clock: Arc::new(固定时钟),
            },
            AgentStatusLeasePolicy::new(
                DurationMillis::new(300_000).expect("租约时长有效"),
                DurationMillis::new(120_000).expect("续租间隔有效"),
                DurationMillis::new(15_000).expect("续租抖动有效"),
            )
            .expect("租约策略有效"),
        ),
        AgentStatusRoomTarget::new(大厅(), AgentStatusVisibility::Coarse),
        HostAgentState::Available,
        matrix.clone(),
    ))
}

fn 身份(name: &str) -> BridgeAgentIdentity {
    BridgeAgentIdentity::new(
        AgentId::from_uuid(
            Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a9a3").expect("Agent 标识有效"),
        ),
        name,
        "@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.test",
        AgentInstanceId::from_uuid(
            Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a9a4").expect("实例标识有效"),
        ),
    )
    .expect("Agent 身份有效")
}

fn 大厅() -> MatrixRoomId {
    MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效")
}

fn 私人房间() -> MatrixRoomId {
    MatrixRoomId::new("!private:matrix.test").expect("房间标识有效")
}

fn 时刻(millis: i64) -> UtcMillis {
    UtcMillis::new(millis).expect("测试时间有效")
}

/// 服务器上这个实例在大厅里的那条状态：名片（`card`）或者旧的租约。
fn 房间里的(name: &str, card: bool) -> Value {
    let mut content = json!({
        "actor": {
            "agent": {
                "agentId": "01945c1e-7b5a-7c7f-8a28-2de53f56a9a3",
                "displayName": name,
                "matrixUserId": "@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.test",
            },
            "instanceId": "01945c1e-7b5a-7c7f-8a28-2de53f56a9a4",
            "provenance": "autonomous_agent",
        },
        "status": "idle",
        "leaseExpiresAt": "1970-01-01T00:05:01Z",
    });
    if card {
        content["liveness"] = json!("presence");
    }
    content
}

/// 只管在线状态的 Matrix：记下报成功的在线状态，读回自己时开着就答最后报的那个。
struct 在线状态网关 {
    metadata: MatrixSessionMetadata,
    /// 没开时读回自己一律离线，和 Synapse 一样。
    enabled: bool,
    report_failures: Mutex<Vec<MatrixFailureKind>>,
    reported: Mutex<Vec<MatrixPresenceState>>,
    /// 服务器上这个实例在大厅里的那条状态。
    room_state: Mutex<Option<Value>>,
    state_failures: Mutex<Vec<MatrixFailureKind>>,
    state_reads: Mutex<usize>,
}

impl 在线状态网关 {
    fn new(enabled: bool) -> Arc<Self> {
        Arc::new(Self {
            metadata: MatrixSessionMetadata::new(
                MatrixUserId::new("@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.test")
                    .expect("Matrix 用户有效"),
                MatrixDeviceId::new("DEVICE").expect("设备有效"),
            ),
            enabled,
            report_failures: Mutex::new(Vec::new()),
            reported: Mutex::new(Vec::new()),
            room_state: Mutex::new(None),
            state_failures: Mutex::new(Vec::new()),
            state_reads: Mutex::new(0),
        })
    }

    fn fail_next_report(&self, kind: MatrixFailureKind) {
        self.report_failures.lock().expect("锁可用").push(kind);
    }

    fn reported(&self) -> Vec<MatrixPresenceState> {
        self.reported.lock().expect("锁可用").clone()
    }
}

impl MatrixGateway for 在线状态网关 {
    fn metadata(&self) -> &MatrixSessionMetadata {
        &self.metadata
    }

    fn sync_once<'a>(
        &'a self,
        _request: &'a MatrixSyncRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixSyncBatch>> {
        unreachable!("测试不同步")
    }

    fn create_room<'a>(
        &'a self,
        _request: &'a MatrixCreateRoom,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        unreachable!("不建房间")
    }

    fn resolve_room_alias<'a>(
        &'a self,
        _alias_localpart: &'a MatrixRoomAliasLocalpart,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        unreachable!("不解析别名")
    }

    fn invite<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        unreachable!("不邀请")
    }

    fn join<'a>(&'a self, _room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>> {
        unreachable!("不进房间")
    }

    fn leave<'a>(&'a self, _room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>> {
        unreachable!("不离开房间")
    }

    fn send_event<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _event: &'a MatrixEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixAcceptedEvent>> {
        unreachable!("不发消息")
    }

    fn send_state_event<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _event: &'a MatrixStateEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        unreachable!("状态事件走记录状态发布器")
    }

    fn send_receipt<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _receipt: &'a MatrixReceipt,
    ) -> PortFuture<'a, MatrixResult<()>> {
        unreachable!("不发回执")
    }

    fn backfill<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _request: &'a MatrixBackfillRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixBackfillPage>> {
        unreachable!("不回填")
    }

    fn user_presence<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<MatrixUserPresence>> {
        let state = if self.enabled {
            self.reported().last().copied().unwrap_or(Offline)
        } else {
            Offline
        };
        Box::pin(async move { Ok(MatrixUserPresence::new(user_id.clone(), state, None)) })
    }

    fn report_presence(&self, presence: MatrixPresenceState) -> PortFuture<'_, MatrixResult<()>> {
        let failure = {
            let mut failures = self.report_failures.lock().expect("锁可用");
            (!failures.is_empty()).then(|| failures.remove(0))
        };
        if failure.is_none() {
            self.reported.lock().expect("锁可用").push(presence);
        }
        Box::pin(async move {
            failure.map_or(Ok(()), |kind| {
                Err(MatrixFailure::new(MatrixOperation::ReportPresence, kind))
            })
        })
    }

    fn state_event<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event_type: &'a MatrixEventType,
        state_key: &'a MatrixStateKey,
    ) -> PortFuture<'a, MatrixResult<Option<Value>>> {
        assert_eq!(room_id, &大厅());
        assert_eq!(event_type.as_str(), AGENT_STATUS_EVENT_TYPE);
        assert_eq!(state_key.as_str(), "01945c1e-7b5a-7c7f-8a28-2de53f56a9a4");
        *self.state_reads.lock().expect("锁可用") += 1;
        let failure = {
            let mut failures = self.state_failures.lock().expect("锁可用");
            (!failures.is_empty()).then(|| failures.remove(0))
        };
        let result = match failure {
            Some(kind) => Err(MatrixFailure::new(MatrixOperation::ReadRoomState, kind)),
            None => Ok(self.room_state.lock().expect("锁可用").clone()),
        };
        Box::pin(async move { result })
    }
}

struct 固定时钟;

impl Clock for 固定时钟 {
    fn now(&self) -> UtcMillis {
        时刻(1_000)
    }
}

struct 测试签名身份;

impl DeviceSigningIdentity for 测试签名身份 {
    fn public_key(&self) -> BridgeCredentialResult<DevicePublicSigningKey> {
        Ok(DevicePublicSigningKey::new(vec![8; 32]).expect("测试公钥有效"))
    }

    fn sign(&self, _message: &[u8]) -> BridgeCredentialResult<DeviceSignature> {
        Ok(DeviceSignature::new(vec![9; 64]).expect("测试签名有效"))
    }
}

#[derive(Default)]
struct 记录状态发布器(Mutex<Vec<MatrixStateEvent>>);

impl 记录状态发布器 {
    fn contents(&self) -> Vec<Value> {
        self.0
            .lock()
            .expect("状态事件锁可用")
            .iter()
            .map(|event| event.content().clone())
            .collect()
    }
}

impl AgentStatusStatePublisher for 记录状态发布器 {
    fn publish<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        event: &'a MatrixStateEvent,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        self.0.lock().expect("状态事件锁可用").push(event.clone());
        Box::pin(async {
            Ok(MatrixEventId::new("$status:matrix.test").expect("事件标识有效"))
        })
    }
}

struct 版本七状态标识;

impl StatusEventIdentifierFactory for 版本七状态标识 {
    fn event_id(&self) -> Uuid {
        Uuid::now_v7()
    }

    fn correlation_id(&self) -> Uuid {
        Uuid::now_v7()
    }
}
