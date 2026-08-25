use std::{
    env,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use agent_room_bridge_ipc::{
    IpcApproveHandoffRequest, IpcHandoffPermission, IpcHandoffPurpose, IpcHandoffSubmission,
    IpcMethod, IpcResponse,
};
use agent_room_bridge_local_adapter::{LocalBridgeClient, bridge_runtime_root};

#[tokio::test]
#[ignore = "需要真实 Bridge、控制平面、Synapse 与当前用户的 OS 安全存储"]
async fn 桌面壳可批准真实一次性交接() {
    let data_root = PathBuf::from(required("AGENT_ROOM_BRIDGE_DATA_DIR"));
    assert!(data_root.is_absolute(), "Bridge 数据目录必须是绝对路径");
    let expires_at_unix_ms = current_unix_millis()
        .checked_add(5 * 60 * 1_000)
        .expect("测试到期时间不得溢出");
    let request = IpcApproveHandoffRequest {
        handoff_id: required("AGENT_ROOM_TEST_HANDOFF_ID"),
        principal_id: required("AGENT_ROOM_TEST_PRINCIPAL_ID"),
        room_id: required("AGENT_ROOM_TEST_ROOM_ID"),
        source_content_id: required("AGENT_ROOM_TEST_SOURCE_CONTENT_ID"),
        target_agent_id: required("AGENT_ROOM_TEST_TARGET_AGENT_ID"),
        target_instance_id: required("AGENT_ROOM_TEST_TARGET_INSTANCE_ID"),
        permissions: vec![
            IpcHandoffPermission::ReadText,
            IpcHandoffPermission::IncludeMetadata,
        ],
        purpose: IpcHandoffPurpose::ReplyDraft,
        expires_at_unix_ms,
    };
    let expected_handoff_id = request.handoff_id.clone();
    let response = LocalBridgeClient::desktop_shell(bridge_runtime_root(&data_root))
        .invoke(IpcMethod::ApproveHandoff(request))
        .await
        .unwrap_or_else(|failure| {
            panic!(
                "桌面批准交接失败 [{}，可重试={}]",
                failure.code(),
                failure.retryable()
            )
        });

    assert!(matches!(
        response,
        IpcResponse::ApprovedHandoff {
            handoff: IpcHandoffSubmission::Submitted { handoff_id, .. }
                | IpcHandoffSubmission::DeliveryUncertain { handoff_id },
        } if handoff_id == expected_handoff_id
    ));
}

fn required(name: &'static str) -> String {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| panic!("缺少真实验收变量 {name}"))
}

fn current_unix_millis() -> i64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时钟不得早于 Unix epoch");
    i64::try_from(elapsed.as_millis()).expect("系统时间不得超出 i64")
}
