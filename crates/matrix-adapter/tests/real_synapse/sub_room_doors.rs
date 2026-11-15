//! 有门的子房间在真实 Synapse 上（`specs/redesign/sub-rooms.md` 的“门”和“加密和谁读得到”，2d-6）。
//!
//! 门在聊天服务器上就是邀请制加上加密，控制面只管谁被邀请、什么时候开加密、什么时候写那一刻：
//!
//! - 有门的子房间建的时候就加密，只邀请房主和建的人。父房间里的人没被请进来（批准、管理员请进来，
//!   在聊天服务器上都是一次邀请）进不去，也读不到里面的话；请进来以后进得去、说得了话。
//! - 装门（没加密的子房间开加密）以后，客户端同步到加密状态，再说的话就是加密的；之前说的照旧是明文。
//! - 拆门、父房间公开时写的那一刻（`room.listing.v1`）房间里的人读得到；再开加密、再写一遍都不出错。

use agent_room_application::ports::{MatrixEventType, MatrixStateKey, PrivateRoomListingMatrix};
use agent_room_domain::time::UtcMillis;

use super::*;

const LISTING_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.room.listing.v1";

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse 私人房间配置"]
async fn 真实_synapse_有门的子房间_没请进来的进不去也读不到_请进来以后进得去() {
    let family = family().await;
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let outsider = managed_user(&family.provisioner, &family.factory).await;
    let outsider_id = outsider.session().metadata().user_id().clone();
    // 他在父房间里，说得了话。
    PrivateRoomMatrixGateway::invite(&*family.provisioner, &family.parent, &outsider_id)
        .await
        .expect("能请他进父房间");
    join_with_retry(outsider.gateway(), &family.parent).await;

    let guarded = guarded_sub_room(&family).await;
    let said = send_with_retry(
        family.member.gateway(),
        &guarded,
        &message_event(unique_value("guarded-said"), "有门的子房间里的一句话"),
    )
    .await;
    encrypted_event_session_id(&base_url, &family.member, &guarded, said.event_id()).await;

    let refused_join = outsider
        .gateway()
        .join(&guarded)
        .await
        .expect_err("没请进来的进不去");
    assert_eq!(refused_join.kind(), MatrixFailureKind::Forbidden);
    assert_removed_member_cannot_read_event(&base_url, &outsider, &guarded, said.event_id()).await;

    // 批准或者管理员请进来：邀请、给发言级别，他就进得去、说得了话。
    PrivateRoomMatrixGateway::invite(&*family.provisioner, &guarded, &outsider_id)
        .await
        .expect("能请他进来");
    family
        .provisioner
        .set_speaking(&guarded, &outsider_id, true)
        .await
        .expect("能给他发言级别");
    join_with_retry(outsider.gateway(), &guarded).await;
    speak(&outsider, &guarded).await;
}

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse 私人房间配置"]
async fn 真实_synapse_装门以后再说的话是加密的_写下的那一刻读得到_重做不出错() {
    let family = family().await;
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let [alpha, _] = &family.sub_rooms;
    let before = send_with_retry(
        family.owner.gateway(),
        alpha,
        &message_event(unique_value("before-door"), "装门以前的一句话"),
    )
    .await;
    assert_ne!(
        raw_event_type(&base_url, &family.owner, alpha, before.event_id()).await,
        "m.room.encrypted",
        "没有门、父房间不加密时是明文"
    );

    for _ in 0..2 {
        PrivateRoomListingMatrix::enable_encryption(&*family.provisioner, alpha)
            .await
            .expect("能开加密，再开一次不出错");
    }
    sync_until_state(family.owner.gateway(), alpha, "m.room.encryption").await;
    let after = send_with_retry(
        family.owner.gateway(),
        alpha,
        &message_event(unique_value("after-door"), "装门以后的一句话"),
    )
    .await;
    encrypted_event_session_id(&base_url, &family.owner, alpha, after.event_id()).await;
    assert_ne!(
        raw_event_type(&base_url, &family.owner, alpha, before.event_id()).await,
        "m.room.encrypted",
        "之前说的照旧是明文"
    );

    let at = UtcMillis::new(1_790_000_000_000).expect("时间有效");
    for _ in 0..2 {
        PrivateRoomListingMatrix::mark_listed(&*family.provisioner, alpha, at)
            .await
            .expect("能写下那一刻，再写一样的不出错");
    }
    let listing = family
        .member
        .gateway()
        .state_event(
            alpha,
            &MatrixEventType::new(LISTING_EVENT_TYPE).expect("事件类型有效"),
            &MatrixStateKey::new("").expect("状态键有效"),
        )
        .await
        .expect("房间里的人读得到")
        .expect("写下了");
    assert_eq!(listing["firstListedAt"], "2026-09-21T14:13:20.000Z");
}

/// 一间有门的子房间：加密，只邀请房主和建的人（这里是成员），两个人都进来了、都能说话。
async fn guarded_sub_room(family: &Family) -> MatrixRoomId {
    let owner_id = family.owner.session().metadata().user_id().clone();
    let room = PrivateRoomMatrixProvisioner::create(
        &*family.provisioner,
        &guarded_creation(&owner_id, &family.member_id),
    )
    .await
    .expect("应用服务能建有门的子房间");
    for person in [&family.owner, &family.member] {
        join_with_retry(person.gateway(), &room).await;
    }
    family
        .provisioner
        .set_speaking_batch(
            &room,
            &[
                PrivateMatrixSpeakingAssignment::new(owner_id, true),
                PrivateMatrixSpeakingAssignment::new(family.member_id.clone(), true),
            ],
        )
        .await
        .expect("能给两个人发言级别");
    room
}

fn guarded_creation(owner: &MatrixUserId, creator: &MatrixUserId) -> PrivateMatrixRoomCreation {
    let alias = MatrixRoomAliasLocalpart::new(format!(
        "agent-room-private-test-{}",
        Uuid::now_v7().simple()
    ))
    .expect("稳定别名有效");
    let request = MatrixCreateRoom::new(
        Some("Guarded sub-room acceptance".to_owned()),
        Some("真实 Synapse 有门的子房间验收".to_owned()),
        MatrixRoomVisibility::Private,
        MatrixRoomPreset::PrivateChat,
        false,
        vec![owner.clone(), creator.clone()],
    )
    .expect("建房请求有效")
    .with_alias_localpart(alias.clone())
    .with_end_to_end_encryption()
    .with_power_profile(MatrixRoomPowerProfile::ManagedPrivate);
    PrivateMatrixRoomCreation::new(request, alias).expect("受管的加密房间请求有效")
}

async fn raw_event_type(
    base_url: &str,
    reader: &MatrixConnection,
    room_id: &MatrixRoomId,
    event_id: &MatrixEventId,
) -> String {
    let response = read_raw_room_event(base_url, reader, room_id, event_id)
        .await
        .error_for_status()
        .expect("房间里的人读得到这条")
        .json::<serde_json::Value>()
        .await
        .expect("事件是 JSON");
    response["type"].as_str().expect("事件有类型").to_owned()
}

/// 同步到这间房里出现这种状态为止：客户端看到了，才照它发。
async fn sync_until_state(gateway: &dyn MatrixGateway, room_id: &MatrixRoomId, event_type: &str) {
    let mut since = None;
    for _ in 0..10 {
        let batch = sync(gateway, since).await;
        if batch.rooms().iter().any(|room| {
            room.room_id() == room_id
                && room
                    .state()
                    .iter()
                    .chain(room.timeline().iter())
                    .any(|event| event.event_type().as_str() == event_type)
        }) {
            return;
        }
        since = Some(batch.next_batch().clone());
    }
    panic!("同步结果里始终没有 {event_type}");
}
