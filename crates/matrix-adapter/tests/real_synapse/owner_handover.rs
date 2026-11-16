//! 房主删除了账户、房间交给了管理员，在真实 Synapse 上（`specs/redesign/room-roles.md` 的“房主不在了
//! （R6）”，R6-4）：
//!
//! - 应用服务写得进 `room.owner_handover.v1`，删除任务重来时写一样的内容不出错；
//! - 房间里的人读得到这条状态，网页照它在对话里写一行；
//! - 房间里的人自己写不进这一类状态（私人房间的 `state_default` 只有应用服务够得着），所以网页只认
//!   状态事件，冒充不了换房主。

use agent_room_application::ports::{OwnerHandoverWrite, PrivateRoomOwnerHandoverGateway};

use super::{sub_rooms::room_creation, *};

const OWNER_HANDOVER_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.room.owner_handover.v1";

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse 私人房间配置"]
async fn 真实_synapse_应用服务写得进交给了谁_写一样的不出错_房间里的人读得到_自己写不进() {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let provisioner = application_service_provisioner(
        &base_url,
        required_environment("AGENT_ROOM_MATRIX_TEST_APPSERVICE_TOKEN"),
    );
    let factory = factory(&base_url, TEST_REQUEST_TIMEOUT, 5);
    // 删除任务跑到这一步时原房主的聊天账户已经停用、离开了房间，这里他只当写进状态里的那个 ID。
    let former_owner = managed_user(&provisioner, &factory).await;
    let admin = managed_user(&provisioner, &factory).await;
    let former_owner_id = former_owner.session().metadata().user_id().clone();
    let admin_id = admin.session().metadata().user_id().clone();
    let room = PrivateRoomMatrixProvisioner::create(
        &provisioner,
        &room_creation(&former_owner_id, &admin_id),
    )
    .await
    .expect("应用服务能建受管的私人房间");
    join_with_retry(admin.gateway(), &room).await;
    // 接手的人能说话，也照样写不进房间状态。
    provisioner
        .set_speaking(&room, &admin_id, true)
        .await
        .expect("能给接手的人发言级别");

    let handover = OwnerHandoverWrite {
        owner: admin_id.clone(),
        former_owner: former_owner_id.clone(),
    };
    for _ in 0..2 {
        provisioner
            .announce_owner_handover(&room, &handover)
            .await
            .expect("应用服务写得进，再写一样的不出错");
    }

    let written = admin
        .gateway()
        .state_event(&room, &event_type(), &empty_state_key())
        .await
        .expect("房间里的人读得到")
        .expect("写下了");
    assert_eq!(written["eventType"], OWNER_HANDOVER_EVENT_TYPE);
    assert_eq!(written["owner"], admin_id.as_str());
    assert_eq!(written["formerOwner"], former_owner_id.as_str());

    let forged = MatrixStateEvent::new(
        event_type(),
        empty_state_key(),
        json!({
            "schemaVersion": "1.0",
            "eventType": OWNER_HANDOVER_EVENT_TYPE,
            "owner": admin_id.as_str(),
            "formerOwner": former_owner_id.as_str(),
        }),
    )
    .expect("状态事件有效");
    let refused = admin
        .gateway()
        .send_state_event(&room, &forged)
        .await
        .expect_err("房间里的人自己写不进");
    assert_eq!(refused.kind(), MatrixFailureKind::Forbidden);
}

fn event_type() -> MatrixEventType {
    MatrixEventType::new(OWNER_HANDOVER_EVENT_TYPE).expect("事件类型有效")
}

fn empty_state_key() -> MatrixStateKey {
    MatrixStateKey::new("").expect("状态键有效")
}
