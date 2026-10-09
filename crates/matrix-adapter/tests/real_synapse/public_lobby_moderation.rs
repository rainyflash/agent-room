//! 公开大厅分成好几个分片时的治理（`specs/public-lobby-watch/design.md` 的“状态”一节）。
//!
//! 分片和真的公开大厅一样由应用服务账号建、用 `public_chat` 预设：谁都能进、谁都能说话。治理要
//! 以它的身份找到消息在哪个分片；禁言只压低那个人，不能把整个大厅的发言门槛抬上去。

use super::*;

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse 治理配置"]
async fn 真实_synapse_公开大厅按分片找消息_禁言不连累旁人_封禁撤销不碰没封过的分片() {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let provisioner = application_service_provisioner(
        &base_url,
        required_environment("AGENT_ROOM_MATRIX_TEST_APPSERVICE_TOKEN"),
    );
    let factory = factory(&base_url, TEST_REQUEST_TIMEOUT, 5);
    let speaker = managed_user(&provisioner, &factory).await;
    let bystander = managed_user(&provisioner, &factory).await;
    let speaker_id = speaker.session().metadata().user_id().clone();
    let busy = create_lobby_shard(&provisioner).await;
    let quiet = create_lobby_shard(&provisioner).await;
    for room in [&busy, &quiet] {
        join_with_retry(speaker.gateway(), room).await;
        join_with_retry(bystander.gateway(), room).await;
    }

    let said = send_with_retry(
        speaker.gateway(),
        &quiet,
        &message_event(unique_value("lobby-said"), "公开大厅里的一句话"),
    )
    .await;
    assert!(
        !ModerationEffectGateway::contains_event(&provisioner, &busy, said.event_id())
            .await
            .expect("别的分片里没有这条消息是正常回答"),
        "消息只在说它的那个分片里"
    );
    assert!(
        ModerationEffectGateway::contains_event(&provisioner, &quiet, said.event_id())
            .await
            .expect("建大厅的应用服务账号读得到分片里的消息"),
    );

    verify_mute_spares_bystanders(
        &provisioner,
        &speaker,
        &bystander,
        &speaker_id,
        [&busy, &quiet],
    )
    .await;
    verify_ban_reversal_per_shard(&provisioner, &speaker, &speaker_id, &busy, &quiet).await;
}

async fn create_lobby_shard(provisioner: &MatrixApplicationServiceProvisioner) -> MatrixRoomId {
    let request = MatrixCreateRoom::new(
        Some(format!("公开大厅分片 {}", Uuid::now_v7().simple())),
        Some("真实 Synapse 公开大厅治理验收".to_owned()),
        MatrixRoomVisibility::Private,
        MatrixRoomPreset::PublicChat,
        false,
        Vec::new(),
    )
    .expect("公开大厅建房请求有效")
    .with_member_writable_state_event_type(
        MatrixEventType::new("io.github.rainyflash.agentroom.agent.status.v1")
            .expect("事件类型有效"),
    );
    RoomProvisioningGateway::create_room(provisioner, &request)
        .await
        .expect("应用服务能建公开大厅分片")
}

/// 两个分片都禁言：被禁言的人哪个分片都说不了，旁人照常说；撤销后他又能说。
async fn verify_mute_spares_bystanders(
    provisioner: &MatrixApplicationServiceProvisioner,
    speaker: &MatrixConnection,
    bystander: &MatrixConnection,
    speaker_id: &MatrixUserId,
    rooms: [&MatrixRoomId; 2],
) {
    let mute = moderation_action(ModerationActionKind::Mute, person_target());
    for room in rooms {
        ModerationEffectGateway::apply(provisioner, &mute, &lobby_target(room, &mute, speaker_id))
            .await
            .expect("公开大厅禁言应成功");
    }
    for room in rooms {
        let denied = speaker
            .gateway()
            .send_event(
                room,
                &message_event(unique_value("lobby-muted"), "禁言期不得发言"),
            )
            .await
            .expect_err("禁言后 Matrix 必须在服务端拒绝");
        assert_eq!(denied.kind(), MatrixFailureKind::Forbidden);
        send_with_retry(
            bystander.gateway(),
            room,
            &message_event(unique_value("lobby-bystander"), "旁人照常说话"),
        )
        .await;
    }
    for room in rooms {
        ModerationEffectGateway::reverse(
            provisioner,
            &mute,
            &lobby_target(room, &mute, speaker_id),
        )
        .await
        .expect("撤销禁言应成功");
        send_with_retry(
            speaker.gateway(),
            room,
            &message_event(unique_value("lobby-unmuted"), "撤销后又能说话"),
        )
        .await;
    }
}

/// 只在一个分片里封禁：之后再踢他不报错；在两个分片上撤销，没封过的那个不碰；公开大厅不发邀请，
/// 他自己就能回来。
async fn verify_ban_reversal_per_shard(
    provisioner: &MatrixApplicationServiceProvisioner,
    speaker: &MatrixConnection,
    speaker_id: &MatrixUserId,
    banned_in: &MatrixRoomId,
    untouched: &MatrixRoomId,
) {
    let target = person_target();
    let ban = moderation_action(ModerationActionKind::Ban, target.clone());
    let kick = moderation_action(ModerationActionKind::Kick, target);
    ModerationEffectGateway::apply(
        provisioner,
        &ban,
        &lobby_target(banned_in, &ban, speaker_id),
    )
    .await
    .expect("封禁应成功");
    ModerationEffectGateway::apply(
        provisioner,
        &kick,
        &lobby_target(banned_in, &kick, speaker_id),
    )
    .await
    .expect("踢已经封禁的人什么也不做");
    assert_eq!(
        membership(provisioner, banned_in, speaker_id).await,
        Some(PrivateMatrixMembership::Banned)
    );
    for room in [banned_in, untouched] {
        ModerationEffectGateway::reverse(provisioner, &ban, &lobby_target(room, &ban, speaker_id))
            .await
            .expect("撤销封禁应成功，没封过的分片里什么也不做");
    }
    assert_eq!(
        membership(provisioner, banned_in, speaker_id).await,
        Some(PrivateMatrixMembership::Left)
    );
    assert_eq!(
        membership(provisioner, untouched, speaker_id).await,
        Some(PrivateMatrixMembership::Joined)
    );
    join_with_retry(speaker.gateway(), banned_in).await;
}

async fn membership(
    provisioner: &MatrixApplicationServiceProvisioner,
    room: &MatrixRoomId,
    user: &MatrixUserId,
) -> Option<PrivateMatrixMembership> {
    PrivateRoomMatrixGateway::membership(provisioner, room, user)
        .await
        .expect("应用服务能读成员状态")
}

fn person_target() -> ModerationTarget {
    ModerationTarget::new(
        ModerationTargetKind::Principal,
        PrincipalId::from_uuid(Uuid::now_v7()).to_string(),
    )
    .expect("主体目标有效")
}

fn lobby_target(
    room: &MatrixRoomId,
    action: &ModerationAction,
    user: &MatrixUserId,
) -> ModerationEffectTarget {
    ModerationEffectTarget {
        matrix_room_id: room.clone(),
        room_kind: RoomCatalogKind::PublicLobby,
        target: action.target().clone(),
        target_matrix_user_id: Some(user.clone()),
    }
}
