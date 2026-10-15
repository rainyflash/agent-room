//! Agent 离开一个房间（`specs/agent-identity/design.md` 第 2 步）：控制面以应用服务的身份替它离开。
//!
//! 已经不在房间里时（离开过一次、被移出、根本没进过）Synapse 怎么回答，决定了离开能不能接着放掉
//! 服务器上的位置：控制面把 403 和 404 当作已经离开。这里确认真实的回答只有成功、403、404 三种。

use agent_room_application::ports::{AgentRoomMembershipFactory, MatrixResult};
use agent_room_domain::rooms::MatrixRoomReference;

use super::{public_lobby_moderation::create_lobby_shard, *};

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse 治理配置"]
async fn 真实_synapse_替_agent_离开_已经不在房间里时只回成功或_403_404() {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let provisioner = application_service_provisioner(
        &base_url,
        required_environment("AGENT_ROOM_MATRIX_TEST_APPSERVICE_TOKEN"),
    );
    let agent = agent_user(&provisioner).await;
    let room = create_lobby_shard(&provisioner).await;
    let reference = MatrixRoomReference::new(room.as_str().to_owned()).expect("房间标识有效");
    let membership =
        AgentRoomMembershipFactory::bind(&provisioner, &agent).expect("应用服务能代表这个 Agent");

    membership.join(&reference).await.expect("替它进房间");
    membership.leave(&reference).await.expect("替它离开");
    assert_already_gone(membership.leave(&reference).await, "离开过再离开");

    membership.join(&reference).await.expect("再进来");
    PrivateRoomMatrixGateway::kick(&provisioner, &room, &agent)
        .await
        .expect("建房的应用服务能移出它");
    assert_already_gone(membership.leave(&reference).await, "被移出以后离开");

    let stranger = agent_user(&provisioner).await;
    let never_joined = AgentRoomMembershipFactory::bind(&provisioner, &stranger)
        .expect("应用服务能代表这个 Agent");
    assert_already_gone(never_joined.leave(&reference).await, "没进过的离开");
}

async fn agent_user(provisioner: &MatrixApplicationServiceProvisioner) -> MatrixUserId {
    let registration = MatrixAgentUserRegistration::new(MatrixAgentLocalpart::from_agent_id(
        AgentId::from_uuid(Uuid::now_v7()),
    ));
    provisioner
        .ensure_user(&registration)
        .await
        .expect("可建立隔离的 Agent 用户")
}

fn assert_already_gone(result: MatrixResult<()>, context: &str) {
    if let Err(failure) = result {
        assert!(
            matches!(
                failure.kind(),
                MatrixFailureKind::Forbidden | MatrixFailureKind::NotFound
            ),
            "{context}：{failure:?}"
        );
    }
}
