//! 私人房间解封（`specs/room-people/design.md` 第 2 步）：Synapse 不让解没封的人（`M_BAD_STATE`），
//! 所以网关先看这个人是不是真封着。解开以后他回到“离开”，之后能再邀请。

use super::{public_lobby_moderation::create_lobby_shard, *};

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse 治理配置"]
async fn 真实_synapse_解封只解真封着的人_解开以后能再邀请() {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let provisioner = application_service_provisioner(
        &base_url,
        required_environment("AGENT_ROOM_MATRIX_TEST_APPSERVICE_TOKEN"),
    );
    let room = create_lobby_shard(&provisioner).await;
    let person = person_user(&provisioner).await;

    PrivateRoomMatrixGateway::unban(&provisioner, &room, &person)
        .await
        .expect("没进过、没封过的解封什么都不做");
    PrivateRoomMatrixGateway::ban(&provisioner, &room, &person)
        .await
        .expect("建房的应用服务能封禁");
    assert_eq!(
        membership(&provisioner, &room, &person).await,
        Some(PrivateMatrixMembership::Banned)
    );

    PrivateRoomMatrixGateway::unban(&provisioner, &room, &person)
        .await
        .expect("能解封");
    assert_eq!(
        membership(&provisioner, &room, &person).await,
        Some(PrivateMatrixMembership::Left)
    );
    PrivateRoomMatrixGateway::unban(&provisioner, &room, &person)
        .await
        .expect("已经解开了再解封算成功，保存失败后的重试靠它");

    PrivateRoomMatrixGateway::invite(&provisioner, &room, &person)
        .await
        .expect("解封以后能再邀请");
    assert_eq!(
        membership(&provisioner, &room, &person).await,
        Some(PrivateMatrixMembership::Invited)
    );
}

async fn person_user(provisioner: &MatrixApplicationServiceProvisioner) -> MatrixUserId {
    let registration = MatrixAgentUserRegistration::new(MatrixAgentLocalpart::from_agent_id(
        AgentId::from_uuid(Uuid::now_v7()),
    ));
    provisioner
        .ensure_user(&registration)
        .await
        .expect("可建立隔离的用户")
}

async fn membership(
    provisioner: &MatrixApplicationServiceProvisioner,
    room: &MatrixRoomId,
    user: &MatrixUserId,
) -> Option<PrivateMatrixMembership> {
    PrivateRoomMatrixGateway::membership(provisioner, room, user)
        .await
        .expect("能读成员状态")
}
