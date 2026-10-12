use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, IdentifierFactory, MatrixResult, MatrixRoomId, MatrixUserId, PortFuture,
        PrivateMatrixMembership, PrivateMatrixRoomCreation, PrivateMatrixSpeakingAssignment,
        PrivateRoomAgentDirectory, PrivateRoomMatrixGateway, PrivateRoomMatrixProvisioner,
        PrivateRoomPrincipalDirectory, PrivateRoomSnapshot, PrivateRoomStore,
    },
    private_rooms::{
        ArchivePrivateRoom, ChangePrivateRoomPermissions, CreatePrivateRoom,
        GovernPrivateRoomMember, InspectPrivateRoom, InvitePrivateRoomMember, ListPrivateRooms,
        ListPrivateRoomsForAccount, PrivateRoomActor, PrivateRoomDependencies,
        PrivateRoomFailureKind, PrivateRoomInvitation, PrivateRoomMembershipAction,
        PrivateRoomRetirement, PrivateRoomService, PrivateRoomUseCases, RenamePrivateRoom,
        TransferPrivateRoomOwnership,
    },
};
use agent_room_domain::{
    identity::Principal,
    ids::{
        AdapterBindingId, AgentCardSnapshotId, AgentId, AgentInstanceId, AutomationGrantId,
        ContentId, DeviceAccessTokenId, DeviceId, DeviceRefreshTokenId, DeviceTokenFamilyId,
        HandoffId, LoginAttemptId, OutboxEventId, PrincipalId, RoomCatalogId, RoomInstanceId,
        RoomReservationId, WebSessionId,
    },
    private_rooms::{PrivateRoomCapability, PrivateRoomMembershipStatus, PrivateRoomPermissions},
    rooms::MatrixRoomReference,
    time::UtcMillis,
    version::AggregateVersion,
};
use uuid::Uuid;

#[tokio::test]
async fn 创建完成前先合并初始发言硬边界再发布真实房间() {
    let fixture = Fixture::new();
    let request = fixture.creation_request(speaker_permissions());

    let snapshot = fixture.service.create(request).await.expect("创建应成功");

    assert_eq!(
        fixture.events(),
        vec!["matrix.create", "matrix.speak.batch", "store.create"]
    );
    assert_eq!(snapshot.catalog().name(), "Architecture room");
    assert_eq!(
        snapshot
            .room()
            .member(fixture.member)
            .expect("受邀成员存在")
            .status(),
        PrivateRoomMembershipStatus::Invited
    );
    let creation = fixture.matrix.creation().expect("应记录真实建房请求");
    assert_eq!(creation.request().invite().len(), 3);
    assert_eq!(
        fixture.matrix.speaking_allowed("@owner:matrix.test"),
        Some(true)
    );
    assert_eq!(
        fixture.matrix.speaking_allowed("@member:matrix.test"),
        Some(true)
    );
    assert_eq!(
        fixture
            .matrix
            .speaking_allowed("@content-authority:matrix.test"),
        Some(false)
    );
}

#[tokio::test]
async fn 列表只返回当前主体受邀或已加入的权威房间() {
    let fixture = Fixture::new();
    fixture
        .service
        .create(fixture.creation_request(speaker_permissions()))
        .await
        .expect("创建应成功");

    let invited = fixture
        .service
        .list(ListPrivateRooms {
            actor: actor(fixture.member, "@member:matrix.test"),
        })
        .await
        .expect("受邀成员可列出房间");
    assert_eq!(invited.len(), 1);

    fixture
        .matrix
        .set_membership(&Fixture::member_matrix(), PrivateMatrixMembership::Left);
    fixture
        .service
        .decline(fixture.member_action())
        .await
        .expect("拒绝邀请应成功");
    let declined = fixture
        .service
        .list(ListPrivateRooms {
            actor: actor(fixture.member, "@member:matrix.test"),
        })
        .await
        .expect("拒绝后的列表应可读");
    assert!(declined.is_empty());
}

#[tokio::test]
async fn 替设备列房间只列账号的_agent_进得去的() {
    let fixture = Fixture::new();
    fixture
        .service
        .create(fixture.creation_request(viewer_permissions()))
        .await
        .expect("创建应成功");
    let service = &fixture.service;
    let listed = move |principal_id| async move {
        service
            .list_for_account(ListPrivateRoomsForAccount { principal_id })
            .await
            .expect("设备列房间应成功")
            .len()
    };
    assert_eq!(listed(fixture.owner).await, 1, "房主已加入、能发言");
    assert_eq!(listed(fixture.member).await, 0, "只是受邀，Agent 进不去");

    fixture
        .matrix
        .set_membership(&Fixture::member_matrix(), PrivateMatrixMembership::Joined);
    fixture
        .service
        .accept(fixture.member_action())
        .await
        .expect("接受应成功");
    assert_eq!(listed(fixture.member).await, 0, "已加入但只能看");

    fixture
        .service
        .update_permissions(ChangePrivateRoomPermissions {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
            permissions: speaker_permissions(),
        })
        .await
        .expect("给发言权应成功");
    assert_eq!(listed(fixture.member).await, 1, "能发言了");

    fixture
        .service
        .archive(ArchivePrivateRoom {
            actor: fixture.owner_actor().into(),
            catalog_id: fixture.catalog,
        })
        .await
        .expect("归档应成功");
    assert_eq!(listed(fixture.owner).await, 0, "归档的房间不再接 Agent");
    assert_eq!(listed(fixture.member).await, 0, "归档的房间不再接 Agent");
}

#[tokio::test]
async fn 本机_agent_替账号建房_房主是人_记下建房的_agent() {
    let fixture = Fixture::new();
    fixture.agents.own(fixture.owner, agent_id(150));
    let request = fixture.agent_creation_request(
        PrivateRoomActor::Device(device(fixture.owner, "@owner:matrix.test")),
        agent_id(150),
    );

    let snapshot = fixture
        .service
        .create(request.clone())
        .await
        .expect("本机 Agent 建房应成功");

    assert_eq!(snapshot.room().owner_principal_id(), fixture.owner);
    assert_eq!(snapshot.room().created_by_agent(), Some(agent_id(150)));
    assert!(snapshot.room().admits_agent_of(fixture.owner));
    let creation = fixture.matrix.creation().expect("应记录真实建房请求");
    assert!(
        creation
            .request()
            .invite()
            .iter()
            .any(|user| user.as_str() == "@owner:matrix.test"),
        "人得在 Matrix 上受邀，第一次打开房间时由客户端加入"
    );
    assert_eq!(
        fixture.matrix.speaking_allowed("@owner:matrix.test"),
        Some(true)
    );

    fixture.clear_events();
    let retried = fixture
        .service
        .create(request)
        .await
        .expect("同一个房间 ID 重试回到同一间");
    assert_eq!(retried.catalog().id(), snapshot.catalog().id());
    assert!(!fixture.events().contains(&"matrix.create"), "重试不再建房");
}

#[tokio::test]
async fn 网络_agent_建房_房主是它自己_不邀请合成主体的_matrix_账号() {
    let fixture = Fixture::new();
    let network = principal_id(3);
    fixture.agents.own(network, agent_id(151));

    let snapshot = fixture
        .service
        .create(fixture.agent_creation_request(
            PrivateRoomActor::NetworkAgent(device(network, "@user-0123abcd:matrix.test")),
            agent_id(151),
        ))
        .await
        .expect("网络 Agent 建房应成功");

    assert_eq!(snapshot.room().owner_principal_id(), network);
    assert_eq!(snapshot.room().created_by_agent(), Some(agent_id(151)));
    assert!(
        snapshot.room().admits_agent_of(network),
        "它的 Agent 按房主的 Agent 进"
    );
    let creation = fixture.matrix.creation().expect("应记录真实建房请求");
    let invited: Vec<&str> = creation
        .request()
        .invite()
        .iter()
        .map(MatrixUserId::as_str)
        .collect();
    assert_eq!(invited, vec!["@content-authority:matrix.test"]);
    assert_eq!(
        fixture
            .matrix
            .speaking_allowed("@user-0123abcd:matrix.test"),
        None
    );
}

#[tokio::test]
async fn agent_建房要用自己名下的_agent_也不顺带邀请人() {
    let fixture = Fixture::new();
    fixture.agents.own(fixture.member, agent_id(152));
    let owner_device = || PrivateRoomActor::Device(device(fixture.owner, "@owner:matrix.test"));

    let not_owned = fixture
        .service
        .create(fixture.agent_creation_request(owner_device(), agent_id(152)))
        .await
        .expect_err("别人名下的 Agent 不能替我建房");
    assert_eq!(not_owned.kind(), PrivateRoomFailureKind::Forbidden);

    let mut without_agent = fixture.agent_creation_request(owner_device(), agent_id(152));
    without_agent.created_by_agent = None;
    let mut with_invitations = fixture.agent_creation_request(owner_device(), agent_id(152));
    with_invitations.invitations = vec![PrivateRoomInvitation {
        principal_id: fixture.member,
        permissions: speaker_permissions(),
    }];
    let mut person_with_agent = fixture.creation_request(speaker_permissions());
    person_with_agent.created_by_agent = Some(agent_id(152));
    for request in [without_agent, with_invitations, person_with_agent] {
        let rejected = fixture
            .service
            .create(request)
            .await
            .expect_err("建房的人和 Agent 对不上时拒绝");
        assert_eq!(rejected.kind(), PrivateRoomFailureKind::InvalidRequest);
    }
    assert!(fixture.matrix.creation().is_none(), "被拒的请求一间也不建");
}

#[tokio::test]
async fn agent_替主体邀请人_只在_agent_建的房间里_只给查看和发言() {
    let fixture = Fixture::new();
    fixture.agents.own(fixture.owner, agent_id(160));
    let owner_device = || PrivateRoomActor::Device(device(fixture.owner, "@owner:matrix.test"));
    fixture
        .service
        .create(fixture.agent_creation_request(owner_device(), agent_id(160)))
        .await
        .expect("Agent 建房应成功");
    let invite = |permissions| InvitePrivateRoomMember {
        actor: owner_device(),
        catalog_id: fixture.catalog,
        target_principal_id: fixture.member,
        permissions,
    };

    let too_little = fixture
        .service
        .invite(invite(viewer_permissions()))
        .await
        .expect_err("Agent 邀请的人只能查看和发言，不多不少");
    assert_eq!(too_little.kind(), PrivateRoomFailureKind::InvalidRequest);

    let invited = fixture
        .service
        .invite(invite(PrivateRoomPermissions::INVITED_BY_AGENT))
        .await
        .expect("Agent 建的房间里能替房主邀请人");
    let member = invited.room().member(fixture.member).expect("受邀");
    assert_eq!(member.status(), PrivateRoomMembershipStatus::Invited);
    assert_eq!(
        member.permissions(),
        PrivateRoomPermissions::INVITED_BY_AGENT
    );
    assert_eq!(
        fixture.matrix.speaking_allowed("@member:matrix.test"),
        Some(true)
    );
}

#[tokio::test]
async fn 人建的房间_agent_不能替人邀请() {
    let fixture = Fixture::new();
    fixture
        .service
        .create(fixture.creation_request(speaker_permissions()))
        .await
        .expect("人建房应成功");
    fixture
        .matrix
        .set_membership(&Fixture::member_matrix(), PrivateMatrixMembership::Left);
    fixture
        .service
        .decline(fixture.member_action())
        .await
        .expect("拒绝邀请应成功");

    let refused = fixture
        .service
        .invite(InvitePrivateRoomMember {
            actor: PrivateRoomActor::Device(device(fixture.owner, "@owner:matrix.test")),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
            permissions: PrivateRoomPermissions::INVITED_BY_AGENT,
        })
        .await
        .expect_err("人建的房间要不要请人由人决定");
    assert_eq!(refused.kind(), PrivateRoomFailureKind::Forbidden);
}

#[tokio::test]
async fn 本机设备不能归档_网络_agent_能关自己的房间() {
    let local = Fixture::new();
    local.agents.own(local.owner, agent_id(161));
    let owner_device = || PrivateRoomActor::Device(device(local.owner, "@owner:matrix.test"));
    local
        .service
        .create(local.agent_creation_request(owner_device(), agent_id(161)))
        .await
        .expect("本机 Agent 建房应成功");
    let refused = local
        .service
        .archive(ArchivePrivateRoom {
            actor: owner_device(),
            catalog_id: local.catalog,
        })
        .await
        .expect_err("本机 Agent 建的房间由人在网页上关");
    assert_eq!(refused.kind(), PrivateRoomFailureKind::Forbidden);

    let network = Fixture::new();
    let network_principal = principal_id(3);
    network.agents.own(network_principal, agent_id(162));
    let network_agent =
        || PrivateRoomActor::NetworkAgent(device(network_principal, "@user-0123abcd:matrix.test"));
    network
        .service
        .create(network.agent_creation_request(network_agent(), agent_id(162)))
        .await
        .expect("网络 Agent 建房应成功");
    let archived = network
        .service
        .archive(ArchivePrivateRoom {
            actor: network_agent(),
            catalog_id: network.catalog,
        })
        .await
        .expect("网络 Agent 能关自己的房间");
    assert_eq!(archived.catalog().status().as_str(), "archived");
}

#[tokio::test]
async fn 网络_agent_停用时系统关掉它当房主的房间_没关成下次接着关_关上的不再动() {
    let fixture = Fixture::new();
    let network_principal = principal_id(3);
    fixture.agents.own(network_principal, agent_id(163));
    fixture
        .service
        .create(fixture.agent_creation_request(
            PrivateRoomActor::NetworkAgent(device(network_principal, "@user-0123abcd:matrix.test")),
            agent_id(163),
        ))
        .await
        .expect("网络 Agent 建房应成功");
    fixture.clear_events();

    fixture.store.fail_next_save.store(true, Ordering::SeqCst);
    fixture
        .service
        .archive_owned_by(network_principal)
        .await
        .expect_err("存不进去就报错，留给下一次");
    fixture.clear_events();
    assert_eq!(
        fixture
            .service
            .archive_owned_by(network_principal)
            .await
            .expect("下一次接着关"),
        1
    );
    assert_eq!(fixture.events(), ["matrix.archive", "store.save"]);
    let stored = fixture.store.snapshot.lock().expect("快照锁正常").clone();
    assert_eq!(
        stored.expect("房间还在").catalog().status().as_str(),
        "archived"
    );

    fixture.clear_events();
    assert_eq!(
        fixture
            .service
            .archive_owned_by(network_principal)
            .await
            .expect("再来一次也不出错"),
        0
    );
    assert!(fixture.events().is_empty(), "关上的不再动 Matrix");
}

#[tokio::test]
async fn 系统只关房主的房间_成员的不动() {
    let fixture = Fixture::new();
    fixture
        .service
        .create(fixture.creation_request(speaker_permissions()))
        .await
        .expect("人建房应成功");
    fixture.clear_events();

    assert_eq!(
        fixture
            .service
            .archive_owned_by(fixture.member)
            .await
            .expect("没有它当房主的房间"),
        0
    );
    assert!(fixture.events().is_empty());
}

#[tokio::test]
async fn 同一个房间_id_换了建房的_agent_算冲突() {
    let fixture = Fixture::new();
    fixture.agents.own(fixture.owner, agent_id(153));
    fixture.agents.own(fixture.owner, agent_id(154));
    let owner_device = || PrivateRoomActor::Device(device(fixture.owner, "@owner:matrix.test"));
    fixture
        .service
        .create(fixture.agent_creation_request(owner_device(), agent_id(153)))
        .await
        .expect("第一次建房应成功");

    let conflict = fixture
        .service
        .create(fixture.agent_creation_request(owner_device(), agent_id(154)))
        .await
        .expect_err("同一个房间 ID 换了建房的 Agent");
    assert_eq!(conflict.kind(), PrivateRoomFailureKind::Conflict);
}

#[tokio::test]
async fn 建房之后邀请的成员按邀请的权限得到或收回发言级别() {
    let fixture = Fixture::new();
    fixture
        .service
        .create(fixture.creation_request(viewer_permissions()))
        .await
        .expect("创建应成功");
    assert_eq!(
        fixture.matrix.speaking_allowed("@member:matrix.test"),
        Some(false)
    );
    let invite = |permissions| InvitePrivateRoomMember {
        actor: fixture.owner_actor().into(),
        catalog_id: fixture.catalog,
        target_principal_id: fixture.member,
        permissions,
    };
    let decline = || async {
        fixture
            .matrix
            .set_membership(&Fixture::member_matrix(), PrivateMatrixMembership::Left);
        fixture
            .service
            .decline(fixture.member_action())
            .await
            .expect("拒绝邀请应成功");
    };

    // 以前只有建房时一起邀请的成员拿到发言级别，之后邀请的人进了房间也发不了言。
    decline().await;
    fixture
        .service
        .invite(invite(speaker_permissions()))
        .await
        .expect("再次邀请应成功");
    assert_eq!(
        fixture.matrix.speaking_allowed("@member:matrix.test"),
        Some(true)
    );

    // 再邀请时只给旁观：收回以前的发言级别。
    decline().await;
    fixture
        .service
        .invite(invite(viewer_permissions()))
        .await
        .expect("只给旁观的邀请应成功");
    assert_eq!(
        fixture.matrix.speaking_allowed("@member:matrix.test"),
        Some(false)
    );
}

#[tokio::test]
async fn 接受邀请必须先由真实_matrix_客户端完成加入() {
    let fixture = Fixture::new();
    fixture
        .service
        .create(fixture.creation_request(speaker_permissions()))
        .await
        .expect("创建应成功");
    fixture.clear_events();

    let rejected = fixture
        .service
        .accept(fixture.member_action())
        .await
        .expect_err("只有 Matrix 邀请态时不能伪装已加入");
    assert_eq!(rejected.kind(), PrivateRoomFailureKind::Conflict);
    assert_eq!(fixture.events(), vec!["matrix.membership"]);
    assert_eq!(
        fixture.store.member_status(fixture.member),
        PrivateRoomMembershipStatus::Invited
    );

    fixture
        .matrix
        .set_membership(&Fixture::member_matrix(), PrivateMatrixMembership::Joined);
    fixture.clear_events();
    let accepted = fixture
        .service
        .accept(fixture.member_action())
        .await
        .expect("Matrix 已加入后可接受");
    assert_eq!(fixture.events(), vec!["matrix.membership", "store.save"]);
    assert_eq!(
        accepted
            .room()
            .member(fixture.member)
            .expect("成员存在")
            .status(),
        PrivateRoomMembershipStatus::Joined
    );
}

#[tokio::test]
async fn 移除先收紧_matrix_再保存且数据库失败不会重新放开访问() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    fixture.store.fail_next_save.store(true, Ordering::SeqCst);
    fixture.clear_events();

    let failure = fixture
        .service
        .remove(GovernPrivateRoomMember {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
        })
        .await
        .expect_err("模拟持久化失败");

    assert_eq!(
        failure.kind(),
        PrivateRoomFailureKind::DependencyUnavailable
    );
    assert_eq!(fixture.events(), vec!["matrix.kick", "store.save"]);
    assert_eq!(
        fixture.matrix.membership_of(&Fixture::member_matrix()),
        Some(PrivateMatrixMembership::Left)
    );
    assert_eq!(
        fixture.store.member_status(fixture.member),
        PrivateRoomMembershipStatus::Joined
    );

    fixture.clear_events();
    let removed = fixture
        .service
        .remove(GovernPrivateRoomMember {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
        })
        .await
        .expect("重试应收敛产品状态");
    assert_eq!(fixture.events(), vec!["matrix.kick", "store.save"]);
    assert_eq!(
        removed
            .room()
            .member(fixture.member)
            .expect("历史成员存在")
            .status(),
        PrivateRoomMembershipStatus::Removed
    );
}

#[tokio::test]
async fn 权限降级先写_matrix_而升级先写产品事实() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    fixture.clear_events();

    fixture
        .service
        .update_permissions(ChangePrivateRoomPermissions {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
            permissions: viewer_permissions(),
        })
        .await
        .expect("降级应成功");
    assert_eq!(fixture.events(), vec!["matrix.speak.off", "store.save"]);

    fixture.clear_events();
    fixture
        .service
        .update_permissions(ChangePrivateRoomPermissions {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
            permissions: speaker_permissions(),
        })
        .await
        .expect("升级应成功");
    assert_eq!(fixture.events(), vec!["store.save", "matrix.speak.on"]);
}

/// 成员带进来的 Agent 已在房间里并拿到发言级别；另一个 Agent 从没进过这个房间。
fn member_agents_entered(fixture: &Fixture) -> (MatrixUserId, MatrixUserId) {
    let in_room = MatrixUserId::new("@_agent_member_in_room:matrix.test").expect("用户有效");
    let elsewhere = MatrixUserId::new("@_agent_member_elsewhere:matrix.test").expect("用户有效");
    fixture.agents.register(fixture.member, &in_room);
    fixture.agents.register(fixture.member, &elsewhere);
    fixture
        .matrix
        .set_membership(&in_room, PrivateMatrixMembership::Joined);
    fixture
        .matrix
        .speaking
        .lock()
        .expect("发言锁正常")
        .insert(in_room.as_str().to_owned(), true);
    (in_room, elsewhere)
}

#[tokio::test]
async fn 成员被收回发言时其_agent_一起收回且恢复时一起恢复() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    let (in_room, elsewhere) = member_agents_entered(&fixture);
    fixture.clear_events();

    fixture
        .service
        .update_permissions(ChangePrivateRoomPermissions {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
            permissions: viewer_permissions(),
        })
        .await
        .expect("降级应成功");

    // Agent 的能力不能超过带它进来的成员：成员不能发言，它也不能。
    assert_eq!(
        fixture.matrix.speaking_allowed(in_room.as_str()),
        Some(false)
    );
    assert_eq!(fixture.matrix.speaking_allowed(elsewhere.as_str()), None);
    // 收紧仍然先写 Matrix，数据库失败时不会留下仍能发言的 Agent。
    assert_eq!(fixture.events().last(), Some(&"store.save"));

    fixture
        .service
        .update_permissions(ChangePrivateRoomPermissions {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
            permissions: speaker_permissions(),
        })
        .await
        .expect("升级应成功");
    assert_eq!(
        fixture.matrix.speaking_allowed(in_room.as_str()),
        Some(true)
    );
    assert_eq!(fixture.matrix.speaking_allowed(elsewhere.as_str()), None);
}

#[tokio::test]
async fn 移除成员时一并移出随其入场的_agent() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    let (in_room, elsewhere) = member_agents_entered(&fixture);

    fixture
        .service
        .remove(GovernPrivateRoomMember {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
        })
        .await
        .expect("移除应成功");

    // 以前这里只移除成员本人，他的 Agent 仍留在房间里读消息。
    assert_eq!(
        fixture.matrix.membership_of(&in_room),
        Some(PrivateMatrixMembership::Left)
    );
    assert_eq!(fixture.matrix.membership_of(&elsewhere), None);
}

#[tokio::test]
async fn 成员自行离开时随其入场的_agent_也离开() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    let (in_room, _) = member_agents_entered(&fixture);
    fixture
        .matrix
        .set_membership(&Fixture::member_matrix(), PrivateMatrixMembership::Left);

    fixture
        .service
        .leave(fixture.member_action())
        .await
        .expect("离开应成功");

    assert_eq!(
        fixture.matrix.membership_of(&in_room),
        Some(PrivateMatrixMembership::Left)
    );
}

#[tokio::test]
async fn 房主转移支持响应丢失后的幂等协议对账() {
    let fixture = Fixture::joined(viewer_permissions()).await;
    let request = TransferPrivateRoomOwnership {
        actor: fixture.owner_actor(),
        catalog_id: fixture.catalog,
        target_principal_id: fixture.member,
        former_owner_permissions: viewer_permissions(),
    };
    fixture.clear_events();

    let transferred = fixture
        .service
        .transfer_ownership(request.clone())
        .await
        .expect("转移应成功");
    assert_eq!(
        fixture.events(),
        vec!["matrix.speak.off", "store.save", "matrix.speak.on"]
    );
    assert_eq!(transferred.room().owner_principal_id(), fixture.member);

    fixture.clear_events();
    let reconciled = fixture
        .service
        .transfer_ownership(request)
        .await
        .expect("响应丢失后的同请求应完成对账");
    assert_eq!(
        fixture.events(),
        vec!["matrix.speak.on", "matrix.speak.off"]
    );
    assert_eq!(reconciled.room().owner_principal_id(), fixture.member);
}

#[tokio::test]
async fn 归档先锁死_matrix_消息再原子归档产品状态() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    fixture.clear_events();

    let archived = fixture
        .service
        .archive(ArchivePrivateRoom {
            actor: fixture.owner_actor().into(),
            catalog_id: fixture.catalog,
        })
        .await
        .expect("归档应成功");

    assert_eq!(fixture.events(), vec!["matrix.archive", "store.save"]);
    assert_eq!(archived.catalog().status().as_str(), "archived");
    assert_eq!(archived.instance().state().as_str(), "archived");
}

#[tokio::test]
async fn 房主改名先改_matrix_房间名再写目录_同名不重复写() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    fixture.clear_events();

    let renamed = fixture
        .service
        .rename(RenamePrivateRoom {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            name: "  Design review  ".to_owned(),
        })
        .await
        .expect("房主可以改名");
    assert_eq!(renamed.catalog().name(), "Design review");
    assert_eq!(fixture.events(), vec!["matrix.set_name", "store.rename"]);
    assert_eq!(
        fixture
            .matrix
            .room_name
            .lock()
            .expect("房间名锁正常")
            .as_deref(),
        Some("Design review")
    );
    assert_eq!(
        fixture
            .service
            .inspect(InspectPrivateRoom {
                actor: fixture.owner_actor(),
                catalog_id: fixture.catalog,
            })
            .await
            .expect("可查看")
            .catalog()
            .name(),
        "Design review"
    );

    // 同名不再写任何地方。
    fixture.clear_events();
    fixture
        .service
        .rename(RenamePrivateRoom {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            name: "Design review".to_owned(),
        })
        .await
        .expect("同名幂等");
    assert!(fixture.events().is_empty());
}

#[tokio::test]
async fn 只有房主能改名且名字不能为空() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    fixture.clear_events();
    let forbidden = fixture
        .service
        .rename(RenamePrivateRoom {
            actor: fixture.member_action().actor,
            catalog_id: fixture.catalog,
            name: "Taken over".to_owned(),
        })
        .await
        .expect_err("成员不能改名");
    assert_eq!(forbidden.kind(), PrivateRoomFailureKind::Forbidden);
    let empty = fixture
        .service
        .rename(RenamePrivateRoom {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            name: "   ".to_owned(),
        })
        .await
        .expect_err("空名字无效");
    assert_eq!(empty.kind(), PrivateRoomFailureKind::InvalidRequest);
    assert!(fixture.events().is_empty());
}

struct Fixture {
    service: PrivateRoomService,
    store: Arc<TestStore>,
    matrix: Arc<TestMatrix>,
    agents: Arc<TestAgentDirectory>,
    events: Arc<Mutex<Vec<&'static str>>>,
    catalog: RoomCatalogId,
    owner: PrincipalId,
    member: PrincipalId,
}

impl Fixture {
    fn new() -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        let store = Arc::new(TestStore::new(events.clone()));
        let matrix = Arc::new(TestMatrix::new(events.clone()));
        let owner = principal_id(1);
        let member = principal_id(2);
        let principals = Arc::new(TestPrincipalDirectory::new(BTreeMap::from([
            (
                owner,
                MatrixUserId::new("@owner:matrix.test").expect("用户有效"),
            ),
            (
                member,
                MatrixUserId::new("@member:matrix.test").expect("用户有效"),
            ),
        ])));
        let runtime = Arc::new(TestRuntime);
        let agents = Arc::new(TestAgentDirectory::default());
        let service = PrivateRoomService::new(PrivateRoomDependencies {
            store: store.clone(),
            matrix_provisioner: matrix.clone(),
            matrix: matrix.clone(),
            principals,
            agents: agents.clone(),
            trusted_matrix_readers: vec![
                MatrixUserId::new("@content-authority:matrix.test").expect("服务身份有效"),
            ],
            identifiers: runtime.clone(),
            clock: runtime,
        });
        Self {
            service,
            store,
            matrix,
            agents,
            events,
            catalog: catalog_id(10),
            owner,
            member,
        }
    }

    async fn joined(permissions: PrivateRoomPermissions) -> Self {
        let fixture = Self::new();
        fixture
            .service
            .create(fixture.creation_request(permissions))
            .await
            .expect("创建应成功");
        fixture
            .matrix
            .set_membership(&Self::member_matrix(), PrivateMatrixMembership::Joined);
        fixture
            .service
            .accept(fixture.member_action())
            .await
            .expect("接受应成功");
        fixture
    }

    fn creation_request(&self, permissions: PrivateRoomPermissions) -> CreatePrivateRoom {
        CreatePrivateRoom {
            actor: self.owner_actor().into(),
            catalog_id: self.catalog,
            name: "Architecture room".to_owned(),
            description: "Private design review".to_owned(),
            retention_days: Some(30),
            invitations: vec![PrivateRoomInvitation {
                principal_id: self.member,
                permissions,
            }],
            created_by_agent: None,
        }
    }

    /// Agent 替主体建房：没有顺带邀请，带着建房的 Agent。
    fn agent_creation_request(
        &self,
        actor: PrivateRoomActor,
        agent_id: AgentId,
    ) -> CreatePrivateRoom {
        CreatePrivateRoom {
            actor,
            catalog_id: self.catalog,
            name: "Weekly sync".to_owned(),
            description: String::new(),
            retention_days: None,
            invitations: Vec::new(),
            created_by_agent: Some(agent_id),
        }
    }

    fn member_action(&self) -> PrivateRoomMembershipAction {
        PrivateRoomMembershipAction {
            actor: actor(self.member, "@member:matrix.test"),
            catalog_id: self.catalog,
        }
    }

    fn owner_actor(&self) -> AuthenticatedPrincipal {
        actor(self.owner, "@owner:matrix.test")
    }

    fn member_matrix() -> MatrixUserId {
        MatrixUserId::new("@member:matrix.test").expect("用户有效")
    }

    fn clear_events(&self) {
        self.events.lock().expect("事件锁正常").clear();
    }

    fn events(&self) -> Vec<&'static str> {
        self.events.lock().expect("事件锁正常").clone()
    }
}

struct TestStore {
    snapshot: Mutex<Option<PrivateRoomSnapshot>>,
    events: Arc<Mutex<Vec<&'static str>>>,
    fail_next_save: AtomicBool,
}

impl TestStore {
    fn new(events: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            snapshot: Mutex::new(None),
            events,
            fail_next_save: AtomicBool::new(false),
        }
    }

    fn member_status(&self, principal_id: PrincipalId) -> PrivateRoomMembershipStatus {
        self.snapshot
            .lock()
            .expect("快照锁正常")
            .as_ref()
            .and_then(|snapshot| snapshot.room().member(principal_id))
            .expect("成员存在")
            .status()
    }
}

impl PrivateRoomStore for TestStore {
    fn create<'a>(
        &'a self,
        snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            self.events.lock().expect("事件锁正常").push("store.create");
            let mut stored = self.snapshot.lock().expect("快照锁正常");
            if stored.is_some() {
                return Err(repository_error(
                    "test.create",
                    RepositoryErrorKind::Conflict,
                ));
            }
            *stored = Some(snapshot.clone());
            Ok(())
        })
    }

    fn find_by_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async move {
            Ok(self
                .snapshot
                .lock()
                .expect("快照锁正常")
                .as_ref()
                .filter(|snapshot| snapshot.catalog().id() == catalog_id)
                .cloned())
        })
    }

    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(async move {
            Ok(self
                .snapshot
                .lock()
                .expect("快照锁正常")
                .as_ref()
                .filter(|snapshot| {
                    snapshot.room().member(principal_id).is_some_and(|member| {
                        matches!(
                            member.status(),
                            PrivateRoomMembershipStatus::Invited
                                | PrivateRoomMembershipStatus::Joined
                        )
                    })
                })
                .cloned()
                .into_iter()
                .collect())
        })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async move {
            Ok(self
                .snapshot
                .lock()
                .expect("快照锁正常")
                .as_ref()
                .filter(|snapshot| snapshot.instance().matrix_room_id() == matrix_room_id)
                .cloned())
        })
    }

    fn save<'a>(
        &'a self,
        room: &'a agent_room_domain::private_rooms::PrivateRoom,
        expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            self.events.lock().expect("事件锁正常").push("store.save");
            if self.fail_next_save.swap(false, Ordering::SeqCst) {
                return Err(repository_error(
                    "test.save",
                    RepositoryErrorKind::Unavailable,
                ));
            }
            let mut stored = self.snapshot.lock().expect("快照锁正常");
            let current = stored
                .as_ref()
                .ok_or_else(|| repository_error("test.save", RepositoryErrorKind::NotFound))?;
            if current.room().version() != expected_version {
                return Err(repository_error("test.save", RepositoryErrorKind::Conflict));
            }
            *stored = Some(
                current
                    .clone()
                    .replacing_room(room.clone())
                    .expect("测试快照应一致"),
            );
            Ok(())
        })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            self.events.lock().expect("事件锁正常").push("store.rename");
            let mut stored = self.snapshot.lock().expect("快照锁正常");
            let current = stored
                .as_ref()
                .ok_or_else(|| repository_error("test.rename", RepositoryErrorKind::NotFound))?;
            *stored = Some(
                current
                    .clone()
                    .renaming(name.to_owned())
                    .expect("测试快照应一致"),
            );
            Ok(())
        })
    }
}

struct TestMatrix {
    events: Arc<Mutex<Vec<&'static str>>>,
    creation: Mutex<Option<PrivateMatrixRoomCreation>>,
    memberships: Mutex<BTreeMap<String, PrivateMatrixMembership>>,
    speaking: Mutex<BTreeMap<String, bool>>,
    room_name: Mutex<Option<String>>,
}

impl TestMatrix {
    fn new(events: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            events,
            creation: Mutex::new(None),
            memberships: Mutex::new(BTreeMap::new()),
            speaking: Mutex::new(BTreeMap::new()),
            room_name: Mutex::new(None),
        }
    }

    fn creation(&self) -> Option<PrivateMatrixRoomCreation> {
        self.creation.lock().expect("建房锁正常").clone()
    }

    fn set_membership(&self, user_id: &MatrixUserId, membership: PrivateMatrixMembership) {
        self.memberships
            .lock()
            .expect("成员锁正常")
            .insert(user_id.as_str().to_owned(), membership);
    }

    fn membership_of(&self, user_id: &MatrixUserId) -> Option<PrivateMatrixMembership> {
        self.memberships
            .lock()
            .expect("成员锁正常")
            .get(user_id.as_str())
            .copied()
    }

    fn record(&self, event: &'static str) {
        self.events.lock().expect("事件锁正常").push(event);
    }

    fn speaking_allowed(&self, user_id: &str) -> Option<bool> {
        self.speaking
            .lock()
            .expect("发言锁正常")
            .get(user_id)
            .copied()
    }
}

impl PrivateRoomMatrixProvisioner for TestMatrix {
    fn create<'a>(
        &'a self,
        creation: &'a PrivateMatrixRoomCreation,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(async move {
            self.record("matrix.create");
            creation.request().invite().iter().for_each(|user_id| {
                self.set_membership(user_id, PrivateMatrixMembership::Invited);
            });
            *self.creation.lock().expect("建房锁正常") = Some(creation.clone());
            Ok(MatrixRoomId::new("!private:matrix.test").expect("房间有效"))
        })
    }
}

impl PrivateRoomMatrixGateway for TestMatrix {
    fn membership<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<Option<PrivateMatrixMembership>>> {
        Box::pin(async move {
            self.record("matrix.membership");
            Ok(self.membership_of(user_id))
        })
    }

    fn invite<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.record("matrix.invite");
            self.set_membership(user_id, PrivateMatrixMembership::Invited);
            Ok(())
        })
    }

    fn kick<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.record("matrix.kick");
            self.set_membership(user_id, PrivateMatrixMembership::Left);
            Ok(())
        })
    }

    fn ban<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.record("matrix.ban");
            self.set_membership(user_id, PrivateMatrixMembership::Banned);
            Ok(())
        })
    }

    fn set_speaking<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
        allowed: bool,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.record(if allowed {
                "matrix.speak.on"
            } else {
                "matrix.speak.off"
            });
            self.speaking
                .lock()
                .expect("发言锁正常")
                .insert(user_id.as_str().to_owned(), allowed);
            Ok(())
        })
    }

    fn set_speaking_batch<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        assignments: &'a [PrivateMatrixSpeakingAssignment],
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.record("matrix.speak.batch");
            let mut speaking = self.speaking.lock().expect("发言锁正常");
            for assignment in assignments {
                speaking.insert(
                    assignment.user_id().as_str().to_owned(),
                    assignment.allowed(),
                );
            }
            Ok(())
        })
    }

    fn archive<'a>(&'a self, _room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.record("matrix.archive");
            Ok(())
        })
    }

    fn set_name<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        name: &'a str,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.record("matrix.set_name");
            *self.room_name.lock().expect("房间名锁正常") = Some(name.to_owned());
            Ok(())
        })
    }
}

/// 主体名下的 Agent；默认为空，只有关心 Agent 的用例才登记。
#[derive(Default)]
struct TestAgentDirectory {
    agents: Mutex<BTreeMap<PrincipalId, Vec<MatrixUserId>>>,
    owned: Mutex<Vec<(PrincipalId, AgentId)>>,
}

impl TestAgentDirectory {
    fn own(&self, principal_id: PrincipalId, agent_id: AgentId) {
        self.owned
            .lock()
            .expect("Agent 锁正常")
            .push((principal_id, agent_id));
    }

    fn register(&self, principal_id: PrincipalId, agent: &MatrixUserId) {
        self.agents
            .lock()
            .expect("Agent 锁正常")
            .entry(principal_id)
            .or_default()
            .push(agent.clone());
    }
}

impl PrivateRoomAgentDirectory for TestAgentDirectory {
    fn agent_matrix_users(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<MatrixUserId>>> {
        let agents = self
            .agents
            .lock()
            .expect("Agent 锁正常")
            .get(&principal_id)
            .cloned()
            .unwrap_or_default();
        Box::pin(async move { Ok(agents) })
    }

    fn owns_active_agent(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        let owned = self
            .owned
            .lock()
            .expect("Agent 锁正常")
            .contains(&(principal_id, agent_id));
        Box::pin(async move { Ok(owned) })
    }
}

struct TestPrincipalDirectory {
    users: BTreeMap<PrincipalId, MatrixUserId>,
}

impl TestPrincipalDirectory {
    fn new(users: BTreeMap<PrincipalId, MatrixUserId>) -> Self {
        Self { users }
    }
}

impl PrivateRoomPrincipalDirectory for TestPrincipalDirectory {
    fn matrix_user_id(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>> {
        Box::pin(async move { Ok(self.users.get(&principal_id).cloned()) })
    }
}

struct TestRuntime;

impl Clock for TestRuntime {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(1_000).expect("时间有效")
    }
}

impl IdentifierFactory for TestRuntime {
    fn principal_id(&self) -> PrincipalId {
        principal_id(101)
    }
    fn login_attempt_id(&self) -> LoginAttemptId {
        LoginAttemptId::from_uuid(uuid(102))
    }
    fn web_session_id(&self) -> WebSessionId {
        WebSessionId::from_uuid(uuid(103))
    }
    fn device_id(&self) -> DeviceId {
        DeviceId::from_uuid(uuid(104))
    }
    fn device_token_family_id(&self) -> DeviceTokenFamilyId {
        DeviceTokenFamilyId::from_uuid(uuid(105))
    }
    fn device_access_token_id(&self) -> DeviceAccessTokenId {
        DeviceAccessTokenId::from_uuid(uuid(106))
    }
    fn device_refresh_token_id(&self) -> DeviceRefreshTokenId {
        DeviceRefreshTokenId::from_uuid(uuid(107))
    }
    fn agent_id(&self) -> AgentId {
        AgentId::from_uuid(uuid(108))
    }
    fn agent_card_snapshot_id(&self) -> AgentCardSnapshotId {
        AgentCardSnapshotId::from_uuid(uuid(109))
    }
    fn adapter_binding_id(&self) -> AdapterBindingId {
        AdapterBindingId::from_uuid(uuid(110))
    }
    fn agent_instance_id(&self) -> AgentInstanceId {
        AgentInstanceId::from_uuid(uuid(111))
    }
    fn room_catalog_id(&self) -> RoomCatalogId {
        catalog_id(112)
    }
    fn room_instance_id(&self) -> RoomInstanceId {
        RoomInstanceId::from_uuid(uuid(113))
    }
    fn room_reservation_id(&self) -> RoomReservationId {
        RoomReservationId::from_uuid(uuid(114))
    }
    fn content_id(&self) -> ContentId {
        ContentId::from_uuid(uuid(115))
    }
    fn handoff_id(&self) -> HandoffId {
        HandoffId::from_uuid(uuid(116))
    }
    fn automation_grant_id(&self) -> AutomationGrantId {
        AutomationGrantId::from_uuid(uuid(117))
    }
    fn outbox_event_id(&self) -> OutboxEventId {
        OutboxEventId::from_uuid(uuid(118))
    }
}

fn actor(principal_id: PrincipalId, matrix_user_id: &str) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id,
        matrix_user_id: matrix_user_id.to_owned(),
        display_name: "Test user".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: UtcMillis::new(100).expect("时间有效"),
        expires_at: UtcMillis::new(2_000).expect("时间有效"),
        recently_authenticated: true,
    }
}

/// 替主体行动的设备身份：本机 Bridge 的签名请求或网络 Agent 的令牌请求都长这样。
fn device(principal_id: PrincipalId, matrix_user_id: &str) -> AuthenticatedDevice {
    AuthenticatedDevice {
        account: agent_room_application::ports::PrincipalAccount {
            principal: Principal::new(principal_id),
            matrix_user_id: matrix_user_id.to_owned(),
            display_name: "Test account".to_owned(),
            avatar_content_id: None,
            locale: "zh-CN".to_owned(),
        },
        device_id: DeviceId::from_uuid(uuid(140)),
        access_token_expires_at: UtcMillis::new(2_000).expect("时间有效"),
    }
}

fn agent_id(value: u128) -> AgentId {
    AgentId::from_uuid(uuid(value))
}

fn viewer_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([PrivateRoomCapability::View]).expect("查看权限有效")
}

fn speaker_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
    ])
    .expect("发言权限有效")
}

fn principal_id(value: u128) -> PrincipalId {
    PrincipalId::from_uuid(uuid(value))
}

fn catalog_id(value: u128) -> RoomCatalogId {
    RoomCatalogId::from_uuid(uuid(value))
}

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn repository_error(operation: &'static str, kind: RepositoryErrorKind) -> RepositoryError {
    RepositoryError::new(operation, kind)
}
