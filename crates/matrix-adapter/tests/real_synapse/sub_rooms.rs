//! 子房间在真实 Synapse 上跟着父房间走（`specs/redesign/sub-rooms.md` 的“对账”和“禁言”，2c-3b）。
//!
//! 子房间在聊天服务器上就是另一间受管的私人房间，服务器不知道它挂在谁下面：跟着父房间变全靠控制面
//! 一间一间照着改。这里查它靠的那几个动作在真实服务器上是不是那个意思：
//!
//! - 对账用的踢人、收发言级别、补邀请只动这一间，重做一遍不出错；
//! - 归档了的子房间里照样能踢人、能补邀请，设一次发言级别却会把归档解开，再锁一次还是锁着；
//! - 父房间的禁言到期时（应用层的到期解除接真实服务器，治理记录在内存里），子房间一起放开，这一间
//!   自己还禁着他的不放。
//!
//! 从“父房间里把人移出”到“子房间里他被踢”的整条链要连着数据库的私人房间服务，这里没有，在无头验收
//! 里查（2k）。房间不加密：查的是成员和发言级别，和加密没有关系。

use std::{slice, sync::Arc};

use agent_room_application::moderation::ModerationExpiryUseCases;
use agent_room_domain::moderation::ModerationActionStatus;

use super::{
    moderation_expiry::{HOUR, MemoryLedger, TestClock, expiry_service, land},
    *,
};

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse 私人房间配置"]
async fn 真实_synapse_子房间里照着父房间收发言级别和踢人_只动这一间_重做不出错() {
    let family = family().await;
    let [alpha, beta] = &family.sub_rooms;
    let member = &family.member_id;

    // 父房间里收了他的发言权：照着在这一间里收。别的房间、这一间里别的人都不受影响。
    for _ in 0..2 {
        family
            .provisioner
            .set_speaking(alpha, member, false)
            .await
            .expect("能收回发言级别，再收一次不出错");
    }
    refused(&family.member, alpha).await;
    speak(&family.member, &family.parent).await;
    speak(&family.member, beta).await;
    speak(&family.owner, alpha).await;

    // 他被移出父房间：这一间里把他踢出去。邀请制的房间，踢出去就回不来；对账重做时再踢一次不算错。
    for _ in 0..2 {
        family
            .provisioner
            .kick(alpha, member)
            .await
            .expect("能踢出去，已经不在里面的再踢一次不出错");
    }
    assert_eq!(
        membership(&family, alpha, member).await,
        Some(PrivateMatrixMembership::Left)
    );
    refused(&family.member, alpha).await;
    let uninvited = family
        .member
        .gateway()
        .join(alpha)
        .await
        .expect_err("没有邀请回不来");
    assert_eq!(uninvited.kind(), MatrixFailureKind::Forbidden);
    // 服务器不知道这几间房是一家：别的子房间要各自踢，这一间踢了，那一间里他还在。
    assert_eq!(
        membership(&family, beta, member).await,
        Some(PrivateMatrixMembership::Joined)
    );
    speak(&family.member, beta).await;

    // 他回到父房间、又进这一间：先设发言级别再邀请（对账和他自己加入都是这个顺序），进来就能说。
    family
        .provisioner
        .set_speaking(alpha, member, true)
        .await
        .expect("不在房间里的人也能先设好发言级别");
    PrivateRoomMatrixGateway::invite(&*family.provisioner, alpha, member)
        .await
        .expect("踢出去的人能再邀请");
    assert_eq!(
        membership(&family, alpha, member).await,
        Some(PrivateMatrixMembership::Invited)
    );
    join_with_retry(family.member.gateway(), alpha).await;
    speak(&family.member, alpha).await;
}

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse 私人房间配置"]
async fn 真实_synapse_归档了的子房间照样踢人和补邀请_设发言级别会把它解开_再锁一次还是锁着() {
    let family = family().await;
    let [archived, open] = &family.sub_rooms;
    let newcomer = managed_user(&family.provisioner, &family.factory).await;
    let newcomer_id = newcomer.session().metadata().user_id().clone();

    // 这一间跟着父房间归档：锁住以后谁也说不了。别的房间不受影响。
    family
        .provisioner
        .archive(archived)
        .await
        .expect("能锁住归档的房间");
    refused(&family.owner, archived).await;
    refused(&family.member, archived).await;
    speak(&family.member, open).await;

    // 归档了的也照样踢：被移出父房间的人不该还留在里面读以前的话。
    family
        .provisioner
        .kick(archived, &family.member_id)
        .await
        .expect("归档了的房间里也能踢人");
    assert_eq!(
        membership(&family, archived, &family.member_id).await,
        Some(PrivateMatrixMembership::Left)
    );

    // 补邀请也行：换了房主以后，新房主没进过的归档子房间靠它。他进得来、说不了，锁还在。
    PrivateRoomMatrixGateway::invite(&*family.provisioner, archived, &newcomer_id)
        .await
        .expect("归档了的房间里也能邀请");
    join_with_retry(newcomer.gateway(), archived).await;
    refused(&newcomer, archived).await;
    refused(&family.owner, archived).await;

    // 设一次发言级别，就把整间房的门槛写回“在用”的那一套：原来能说的人又能说了，归档等于解开。
    // 所以对账在归档了的子房间里不设发言级别。哪天这一句不成立了（设发言级别不再碰门槛），那条
    // 规矩可以放宽，这里跟着改。
    family
        .provisioner
        .set_speaking(archived, &newcomer_id, false)
        .await
        .expect("能设发言级别");
    speak(&family.owner, archived).await;

    // 万一设过（做的当口这一间才归档的），再锁一次：还是谁也说不了。
    family
        .provisioner
        .archive(archived)
        .await
        .expect("能再锁一次");
    refused(&family.owner, archived).await;
    refused(&newcomer, archived).await;
}

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse 治理配置"]
async fn 真实_synapse_父房间的禁言到期_子房间一起放开_这一间自己还禁着他的不放() {
    let family = family().await;
    let [alpha, beta] = &family.sub_rooms;
    let everywhere = [family.parent.clone(), alpha.clone(), beta.clone()];
    let ledger = Arc::new(MemoryLedger::family(
        family.parent.clone(),
        &family.sub_rooms,
        family.member_id.clone(),
    ));
    let clock = Arc::new(TestClock::starting());
    let service = expiry_service(&ledger, family.provisioner.clone(), &clock);

    // 父房间里一条一小时的禁言，落的时候连他在的两间子房间一起压。第一间自己另有一条两小时的。
    let above = ledger.applied_mute(&clock, Some(HOUR));
    land(&family.provisioner, &ledger, &above, &everywhere).await;
    let own = ledger.applied_sub_room_mute(0, &clock, Some(2 * HOUR));
    land(&family.provisioner, &ledger, &own, slice::from_ref(alpha)).await;
    for room in &everywhere {
        refused(&family.member, room).await;
        speak(&family.owner, room).await;
    }

    // 父房间的那条到期：父房间和第二间放开；第一间自己还禁着他，不放。
    clock.advance(HOUR);
    let outcome = service.expire_due_actions().await.expect("到期解除应跑完");
    assert_eq!((outcome.expired, outcome.retrying.len()), (1, 0));
    assert_eq!(ledger.status(&above), ModerationActionStatus::Reversed);
    assert_eq!(ledger.status(&own), ModerationActionStatus::Applied);
    speak(&family.member, &family.parent).await;
    speak(&family.member, beta).await;
    refused(&family.member, alpha).await;

    // 第一间自己的那条也到期：父房间的已经没有了，这一间才放开。
    clock.advance(HOUR);
    let outcome = service.expire_due_actions().await.expect("到期解除应跑完");
    assert_eq!((outcome.expired, outcome.retrying.len()), (1, 0));
    assert_eq!(ledger.status(&own), ModerationActionStatus::Reversed);
    speak(&family.member, alpha).await;

    // 反过来：父房间里不限时地禁着他，第一间自己的一小时到期了，这一间也不放。
    let standing = ledger.applied_mute(&clock, None);
    land(&family.provisioner, &ledger, &standing, &everywhere).await;
    let brief = ledger.applied_sub_room_mute(0, &clock, Some(HOUR));
    land(&family.provisioner, &ledger, &brief, slice::from_ref(alpha)).await;
    clock.advance(HOUR);
    let outcome = service.expire_due_actions().await.expect("到期解除应跑完");
    assert_eq!((outcome.expired, outcome.retrying.len()), (1, 0));
    assert_eq!(ledger.status(&brief), ModerationActionStatus::Reversed);
    assert_eq!(ledger.status(&standing), ModerationActionStatus::Applied);
    refused(&family.member, alpha).await;
    refused(&family.member, &family.parent).await;
}

/// 一间父房间和它下面的两间子房间，房主和一个成员在三间里都已加入、都能说话。
struct Family {
    provisioner: Arc<MatrixApplicationServiceProvisioner>,
    factory: MatrixSdkClientFactory,
    owner: MatrixConnection,
    member: MatrixConnection,
    member_id: MatrixUserId,
    parent: MatrixRoomId,
    sub_rooms: [MatrixRoomId; 2],
}

async fn family() -> Family {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let provisioner = Arc::new(application_service_provisioner(
        &base_url,
        required_environment("AGENT_ROOM_MATRIX_TEST_APPSERVICE_TOKEN"),
    ));
    let factory = factory(&base_url, TEST_REQUEST_TIMEOUT, 5);
    let owner = managed_user(&provisioner, &factory).await;
    let member = managed_user(&provisioner, &factory).await;
    let owner_id = owner.session().metadata().user_id().clone();
    let member_id = member.session().metadata().user_id().clone();
    let mut rooms = Vec::new();
    for _ in 0..3 {
        let room = PrivateRoomMatrixProvisioner::create(
            &*provisioner,
            &room_creation(&owner_id, &member_id),
        )
        .await
        .expect("应用服务能建受管的私人房间");
        for person in [&owner, &member] {
            join_with_retry(person.gateway(), &room).await;
        }
        provisioner
            .set_speaking_batch(
                &room,
                &[
                    PrivateMatrixSpeakingAssignment::new(owner_id.clone(), true),
                    PrivateMatrixSpeakingAssignment::new(member_id.clone(), true),
                ],
            )
            .await
            .expect("能给两个人发言级别");
        rooms.push(room);
    }
    let [parent, alpha, beta] = <[MatrixRoomId; 3]>::try_from(rooms).expect("建了三间");
    Family {
        provisioner,
        factory,
        owner,
        member,
        member_id,
        parent,
        sub_rooms: [alpha, beta],
    }
}

/// 一间受管的私人房间，邀请制、不加密。子房间和父房间在聊天服务器上是同一种房间。
fn room_creation(owner: &MatrixUserId, member: &MatrixUserId) -> PrivateMatrixRoomCreation {
    let alias = MatrixRoomAliasLocalpart::new(format!(
        "agent-room-private-test-{}",
        Uuid::now_v7().simple()
    ))
    .expect("稳定别名有效");
    let request = MatrixCreateRoom::new(
        Some("Sub-room acceptance".to_owned()),
        Some("真实 Synapse 子房间验收".to_owned()),
        MatrixRoomVisibility::Private,
        MatrixRoomPreset::PrivateChat,
        false,
        vec![owner.clone(), member.clone()],
    )
    .expect("建房请求有效")
    .with_alias_localpart(alias.clone())
    .with_power_profile(MatrixRoomPowerProfile::ManagedPrivate);
    PrivateMatrixRoomCreation::new_public(request, alias).expect("受管的不加密房间请求有效")
}

async fn membership(
    family: &Family,
    room: &MatrixRoomId,
    user: &MatrixUserId,
) -> Option<PrivateMatrixMembership> {
    PrivateRoomMatrixGateway::membership(&*family.provisioner, room, user)
        .await
        .expect("应用服务能读成员状态")
}

/// 这个人在这间房里说得出一句话。
async fn speak(who: &MatrixConnection, room: &MatrixRoomId) {
    send_with_retry(
        who.gateway(),
        room,
        &message_event(unique_value("sub-room-said"), "子房间验收里的一句话"),
    )
    .await;
}

/// 这个人在这间房里说不了话：服务器自己拒的。一个人连着发得多了会先被限速，等一等再试；别的失败
/// 直接算错。
async fn refused(who: &MatrixConnection, room: &MatrixRoomId) {
    let policy = retry_policy();
    for completed_attempts in 1..=5 {
        let failure = who
            .gateway()
            .send_event(
                room,
                &message_event(unique_value("sub-room-refused"), "这一句不该发得出去"),
            )
            .await
            .expect_err("这一句不该发得出去");
        if failure.kind() == MatrixFailureKind::Forbidden {
            return;
        }
        wait_for_retry(policy, failure, completed_attempts).await;
    }
    panic!("一直被限速，没等到服务器的拒绝")
}
