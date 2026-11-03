//! 台账里私人房间的踢出、封禁照成员一节那一套落（`specs/public-rooms/design.md`“落到哪”）：改成员记录、
//! 在聊天服务器上踢出或封禁、把他名下的 Agent 移出；撤回封禁是解封，撤回踢出是照新成员的默认权限请他回来。

use agent_room_application::ports::{ModerationMemberGateway, ModerationMemberRefusal};
use agent_room_domain::{
    join_codes::PrivateRoomAgentMemberStatus, moderation::ModerationActionKind,
};

use super::*;

#[tokio::test]
async fn 台账的踢出照成员一节移出他_连同他的_agent() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    let (in_room, elsewhere) = member_agents_entered(&fixture);

    ModerationMemberGateway::apply(
        &fixture.service,
        ModerationActionKind::Kick,
        fixture.catalog,
        fixture.member,
    )
    .await
    .expect("踢出");

    assert_eq!(
        fixture.store.member_status(fixture.member),
        PrivateRoomMembershipStatus::Removed
    );
    assert_eq!(
        fixture.matrix.membership_of(&Fixture::member_matrix()),
        Some(PrivateMatrixMembership::Left)
    );
    assert_eq!(
        fixture.matrix.membership_of(&in_room),
        Some(PrivateMatrixMembership::Left)
    );
    assert_eq!(fixture.matrix.membership_of(&elsewhere), None);
}

#[tokio::test]
async fn 台账的封禁照成员一节封禁_撤回是解封_回到已移出() {
    let fixture = Fixture::joined(speaker_permissions()).await;

    ModerationMemberGateway::apply(
        &fixture.service,
        ModerationActionKind::Ban,
        fixture.catalog,
        fixture.member,
    )
    .await
    .expect("封禁");
    assert_eq!(
        fixture.store.member_status(fixture.member),
        PrivateRoomMembershipStatus::Banned
    );

    fixture.clear_events();
    ModerationMemberGateway::reverse(
        &fixture.service,
        ModerationActionKind::Ban,
        fixture.catalog,
        fixture.member,
    )
    .await
    .expect("解封");
    assert_eq!(
        fixture.store.member_status(fixture.member),
        PrivateRoomMembershipStatus::Removed
    );
    assert_eq!(fixture.events(), vec!["matrix.unban", "store.save"]);
}

#[tokio::test]
async fn 撤回踢出照新成员的默认权限请他回来_已经请了的不再请() {
    let fixture = Fixture::joined(viewer_permissions()).await;
    ModerationMemberGateway::apply(
        &fixture.service,
        ModerationActionKind::Kick,
        fixture.catalog,
        fixture.member,
    )
    .await
    .expect("踢出");

    let reverse = || {
        ModerationMemberGateway::reverse(
            &fixture.service,
            ModerationActionKind::Kick,
            fixture.catalog,
            fixture.member,
        )
    };
    reverse().await.expect("请回来");
    assert_eq!(
        fixture.store.member_status(fixture.member),
        PrivateRoomMembershipStatus::Invited
    );
    assert_eq!(
        fixture.matrix.membership_of(&Fixture::member_matrix()),
        Some(PrivateMatrixMembership::Invited)
    );
    // 没公开过的房间和邀请链接一样给查看、发言、自动发言，不是他原来只能看的权限。
    assert_eq!(
        fixture.matrix.speaking_allowed("@member:matrix.test"),
        Some(true)
    );

    fixture.clear_events();
    reverse().await.expect("已经请了");
    assert!(fixture.events().is_empty(), "{:?}", fixture.events());
}

#[tokio::test]
async fn 落之前先看做不做得了() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    let check = |kind, catalog_id, target| {
        ModerationMemberGateway::check(&fixture.service, kind, catalog_id, target)
    };

    assert_eq!(
        check(ModerationActionKind::Ban, fixture.catalog, fixture.owner)
            .await
            .unwrap(),
        Some(ModerationMemberRefusal::Owner)
    );
    assert_eq!(
        check(ModerationActionKind::Kick, fixture.catalog, fixture.member)
            .await
            .unwrap(),
        None
    );
    let stranger = principal_id(99);
    assert_eq!(
        check(ModerationActionKind::Kick, fixture.catalog, stranger)
            .await
            .unwrap(),
        Some(ModerationMemberRefusal::NotInRoom)
    );
    // 不在房间里的人也封得了：封着，以后进不来。
    assert_eq!(
        check(ModerationActionKind::Ban, fixture.catalog, stranger)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        check(ModerationActionKind::Ban, catalog_id(77), fixture.member)
            .await
            .unwrap(),
        Some(ModerationMemberRefusal::RoomUnavailable)
    );
}

/// 网络 Agent（台账里是替它记账的合成主体 50）凭口令进了房间。
fn network_agent_entered(fixture: &Fixture) -> (PrincipalId, AgentId, MatrixUserId) {
    let principal = principal_id(50);
    let agent = agent_id(51);
    let user = MatrixUserId::new("@_network_agent:matrix.test").expect("用户有效");
    fixture.agents.own(principal, agent);
    fixture
        .agents
        .network
        .lock()
        .expect("Agent 锁正常")
        .push(principal);
    fixture.agent_members.join(fixture.catalog, agent, &user);
    fixture
        .matrix
        .set_membership(&user, PrivateMatrixMembership::Joined);
    (principal, agent, user)
}

#[tokio::test]
async fn 网络_agent_被封禁只移出它这个_agent_成员_不留人的记录_撤回不替它恢复() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    let (network, agent, user) = network_agent_entered(&fixture);
    let gateway = &fixture.service;
    assert_eq!(
        ModerationMemberGateway::check(
            gateway,
            ModerationActionKind::Ban,
            fixture.catalog,
            network
        )
        .await
        .unwrap(),
        None
    );

    ModerationMemberGateway::apply(gateway, ModerationActionKind::Ban, fixture.catalog, network)
        .await
        .expect("封禁");

    assert_eq!(
        fixture.agent_members.status(agent),
        PrivateRoomAgentMemberStatus::Removed
    );
    assert_eq!(
        fixture.matrix.membership_of(&user),
        Some(PrivateMatrixMembership::Left)
    );
    assert!(
        fixture
            .store
            .snapshot
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .room()
            .member(network)
            .is_none(),
        "网络 Agent 不是房间里的人"
    );
    // 已经移出了：再踢、再封都没有可落的。
    for kind in [ModerationActionKind::Kick, ModerationActionKind::Ban] {
        assert_eq!(
            ModerationMemberGateway::check(gateway, kind, fixture.catalog, network)
                .await
                .unwrap(),
            Some(ModerationMemberRefusal::NotInRoom)
        );
    }

    fixture.clear_events();
    ModerationMemberGateway::reverse(gateway, ModerationActionKind::Ban, fixture.catalog, network)
        .await
        .expect("撤回");
    assert!(fixture.events().is_empty(), "{:?}", fixture.events());
    assert_eq!(
        fixture.agent_members.status(agent),
        PrivateRoomAgentMemberStatus::Removed
    );
}

#[tokio::test]
async fn 踢出一个人连他凭口令进来的_agent_成员一起移出_他不在房间里也踢得到他的_agent() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    let code_agent = agent_id(61);
    let code_user = MatrixUserId::new("@_code_agent:matrix.test").expect("用户有效");
    fixture.agents.own(fixture.member, code_agent);
    fixture
        .agent_members
        .join(fixture.catalog, code_agent, &code_user);
    fixture
        .matrix
        .set_membership(&code_user, PrivateMatrixMembership::Joined);
    // 房间外的一个人，他的 Agent 凭口令进来了。
    let stranger = principal_id(70);
    let stranger_agent = agent_id(71);
    let stranger_user = MatrixUserId::new("@_stranger_agent:matrix.test").expect("用户有效");
    fixture.agents.own(stranger, stranger_agent);
    fixture
        .agent_members
        .join(fixture.catalog, stranger_agent, &stranger_user);
    fixture
        .matrix
        .set_membership(&stranger_user, PrivateMatrixMembership::Joined);
    let gateway = &fixture.service;

    for target in [fixture.member, stranger] {
        assert_eq!(
            ModerationMemberGateway::check(
                gateway,
                ModerationActionKind::Kick,
                fixture.catalog,
                target
            )
            .await
            .unwrap(),
            None
        );
        ModerationMemberGateway::apply(
            gateway,
            ModerationActionKind::Kick,
            fixture.catalog,
            target,
        )
        .await
        .expect("踢出");
    }

    assert_eq!(
        fixture.store.member_status(fixture.member),
        PrivateRoomMembershipStatus::Removed
    );
    for (agent, user) in [(code_agent, &code_user), (stranger_agent, &stranger_user)] {
        assert_eq!(
            fixture.agent_members.status(agent),
            PrivateRoomAgentMemberStatus::Removed
        );
        assert_eq!(
            fixture.matrix.membership_of(user),
            Some(PrivateMatrixMembership::Left)
        );
    }
}
