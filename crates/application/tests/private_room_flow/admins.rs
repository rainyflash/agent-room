//! 只有房主能设、撤管理员，管理员之间谁也动不了谁（`specs/redesign/room-roles.md` R1）。规则在领域层，
//! 这里看用例：被拒时报的是“只有房主能做”，聊天服务器和存储都没动；管理员自己不当了做得到。
//! 设、撤管理员还要最近登录过（R2）：登录有效但不是最近的，房主也做不成，别的权限照改。

use agent_room_application::ports::ModerationMemberGateway;

use super::*;

#[tokio::test]
async fn 管理员动不了另一位管理员_被拒时聊天服务器和存储都没动() {
    let fixture = Fixture::joined(admin_permissions()).await;
    let other = principal_id(3);
    seat(&fixture, other, admin_permissions());
    fixture.clear_events();

    let refused = fixture
        .service
        .update_permissions(ChangePrivateRoomPermissions {
            actor: fixture.admin_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: other,
            permissions: speaker_permissions(),
        })
        .await
        .expect_err("管理员撤不了另一位管理员");
    assert_eq!(refused.kind(), PrivateRoomFailureKind::OwnerOnly);

    for govern in [Govern::Remove, Govern::Ban] {
        let request = GovernPrivateRoomMember {
            actor: fixture.admin_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: other,
        };
        let refused = match govern {
            Govern::Remove => fixture.service.remove(request).await,
            Govern::Ban => fixture.service.ban(request).await,
        }
        .expect_err("管理员移不出、封不了另一位管理员");
        assert_eq!(refused.kind(), PrivateRoomFailureKind::OwnerOnly);
    }

    assert!(fixture.events().is_empty(), "{:?}", fixture.events());
    assert!(stored_room(&fixture).is_admin(other));
    assert_eq!(
        fixture.store.member_status(other),
        PrivateRoomMembershipStatus::Joined
    );
}

#[tokio::test]
async fn 管理员设不了管理员_邀请时也带不上管理() {
    let fixture = Fixture::joined(admin_permissions()).await;
    let ordinary = principal_id(3);
    seat(&fixture, ordinary, speaker_permissions());
    fixture.clear_events();

    let refused = fixture
        .service
        .update_permissions(ChangePrivateRoomPermissions {
            actor: fixture.admin_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: ordinary,
            permissions: admin_permissions(),
        })
        .await
        .expect_err("管理员设不了管理员");
    assert_eq!(refused.kind(), PrivateRoomFailureKind::OwnerOnly);

    let refused = fixture
        .service
        .invite(InvitePrivateRoomMember {
            actor: fixture.admin_actor().into(),
            catalog_id: fixture.catalog,
            target_principal_id: principal_id(4),
            permissions: admin_permissions(),
        })
        .await
        .expect_err("管理员请不进一位管理员");
    assert_eq!(refused.kind(), PrivateRoomFailureKind::OwnerOnly);

    assert!(fixture.events().is_empty(), "{:?}", fixture.events());
    assert!(!stored_room(&fixture).is_admin(ordinary));
    assert!(stored_room(&fixture).member(principal_id(4)).is_none());
}

#[tokio::test]
async fn 管理员自己可以不当了_发言照旧() {
    let fixture = Fixture::joined(admin_permissions()).await;
    fixture.clear_events();

    fixture
        .service
        .update_permissions(ChangePrivateRoomPermissions {
            actor: fixture.admin_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
            permissions: speaker_permissions(),
        })
        .await
        .expect("自己把自己降成成员");

    // 发言没变，聊天服务器上不用动。
    assert_eq!(fixture.events(), vec!["store.save"]);
    assert!(!stored_room(&fixture).is_admin(fixture.member));
    assert!(stored_room(&fixture).allows(fixture.member, PrivateRoomCapability::Speak));
}

#[tokio::test]
async fn 房主设得了撤得了管理员() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    let change = |permissions| {
        fixture
            .service
            .update_permissions(ChangePrivateRoomPermissions {
                actor: fixture.owner_actor(),
                catalog_id: fixture.catalog,
                target_principal_id: fixture.member,
                permissions,
            })
    };

    change(admin_permissions()).await.expect("房主设管理员");
    assert!(stored_room(&fixture).is_admin(fixture.member));
    change(speaker_permissions()).await.expect("房主撤管理员");
    assert!(!stored_room(&fixture).is_admin(fixture.member));

    change(admin_permissions()).await.expect("房主再设一次");
    fixture
        .service
        .remove(GovernPrivateRoomMember {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
        })
        .await
        .expect("房主移得出管理员");
    assert_eq!(
        fixture.store.member_status(fixture.member),
        PrivateRoomMembershipStatus::Removed
    );
}

#[tokio::test]
async fn 房主没最近登录过设不了撤不了管理员_别的权限照改() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    fixture.clear_events();
    let change = |actor, permissions| {
        fixture
            .service
            .update_permissions(ChangePrivateRoomPermissions {
                actor,
                catalog_id: fixture.catalog,
                target_principal_id: fixture.member,
                permissions,
            })
    };
    let stale_owner = || stale(fixture.owner_actor());

    let refused = change(stale_owner(), admin_permissions())
        .await
        .expect_err("设管理员要最近登录过");
    assert_eq!(
        refused.kind(),
        PrivateRoomFailureKind::ReauthenticationRequired
    );
    assert!(fixture.events().is_empty(), "{:?}", fixture.events());
    assert!(!stored_room(&fixture).is_admin(fixture.member));

    // 不碰“管理”的照旧只要有效登录。
    change(stale_owner(), inviter_permissions())
        .await
        .expect("改成员的别的权限");
    assert!(stored_room(&fixture).allows(fixture.member, PrivateRoomCapability::Invite));

    change(fixture.owner_actor(), admin_permissions())
        .await
        .expect("最近登录过的房主设管理员");
    fixture.clear_events();
    let refused = change(stale_owner(), speaker_permissions())
        .await
        .expect_err("撤管理员要最近登录过");
    assert_eq!(
        refused.kind(),
        PrivateRoomFailureKind::ReauthenticationRequired
    );
    assert!(fixture.events().is_empty(), "{:?}", fixture.events());
    assert!(stored_room(&fixture).is_admin(fixture.member));

    change(stale_owner(), manager_without_invite())
        .await
        .expect("改管理员的别的权限");
    assert!(stored_room(&fixture).is_admin(fixture.member));
    assert!(!stored_room(&fixture).allows(fixture.member, PrivateRoomCapability::Invite));
}

#[tokio::test]
async fn 管理员自己不当了不用最近登录过() {
    let fixture = Fixture::joined(admin_permissions()).await;

    fixture
        .service
        .update_permissions(ChangePrivateRoomPermissions {
            actor: stale(fixture.admin_actor()),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
            permissions: speaker_permissions(),
        })
        .await
        .expect("往下降不用重新登录");
    assert!(!stored_room(&fixture).is_admin(fixture.member));
}

#[tokio::test]
async fn 请一位管理员进来要最近登录过_请成员不用() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    fixture
        .service
        .remove(GovernPrivateRoomMember {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
        })
        .await
        .expect("先移出，下面再请回来");
    fixture.clear_events();
    let invite = |permissions| {
        fixture.service.invite(InvitePrivateRoomMember {
            actor: stale(fixture.owner_actor()).into(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
            permissions,
        })
    };

    let refused = invite(admin_permissions())
        .await
        .expect_err("请管理员要最近登录过");
    assert_eq!(
        refused.kind(),
        PrivateRoomFailureKind::ReauthenticationRequired
    );
    assert!(fixture.events().is_empty(), "{:?}", fixture.events());
    assert_eq!(
        fixture.store.member_status(fixture.member),
        PrivateRoomMembershipStatus::Removed
    );

    invite(speaker_permissions())
        .await
        .expect("请成员只要有效登录");
    assert_eq!(
        fixture.store.member_status(fixture.member),
        PrivateRoomMembershipStatus::Invited
    );
}

#[tokio::test]
async fn 台账问得出对象是不是只有房主能动() {
    let fixture = Fixture::joined(admin_permissions()).await;
    let other = principal_id(3);
    let ordinary = principal_id(4);
    seat(&fixture, other, admin_permissions());
    seat(&fixture, ordinary, speaker_permissions());
    let owner_only = |actor, target| {
        ModerationMemberGateway::owner_only(&fixture.service, fixture.catalog, actor, target)
    };

    assert!(owner_only(fixture.member, other).await.expect("读得到房间"));
    assert!(
        owner_only(fixture.member, fixture.member)
            .await
            .expect("读得到房间"),
        "自己身上的也只有房主撤得了"
    );
    assert!(!owner_only(fixture.owner, other).await.expect("读得到房间"));
    assert!(
        !owner_only(fixture.member, ordinary)
            .await
            .expect("读得到房间")
    );
    assert!(
        !ModerationMemberGateway::owner_only(
            &fixture.service,
            catalog_id(99),
            fixture.member,
            other
        )
        .await
        .expect("读得到目录"),
        "房间不在了不归这一条管"
    );
}

enum Govern {
    Remove,
    Ban,
}

impl Fixture {
    /// 夹具里的那位成员，这几条用例里他是管理员。
    fn admin_actor(&self) -> AuthenticatedPrincipal {
        actor(self.member, "@member:matrix.test")
    }
}

fn admin_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
        PrivateRoomCapability::Invite,
        PrivateRoomCapability::Manage,
    ])
    .expect("管理员权限有效")
}

fn inviter_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
        PrivateRoomCapability::Invite,
    ])
    .expect("能邀请的成员权限有效")
}

fn manager_without_invite() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
        PrivateRoomCapability::Manage,
    ])
    .expect("不能邀请的管理员权限有效")
}

/// 登录还有效，只是不算最近登录过。
fn stale(actor: AuthenticatedPrincipal) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        recently_authenticated: false,
        ..actor
    }
}

fn stored_room(fixture: &Fixture) -> agent_room_domain::private_rooms::PrivateRoom {
    fixture
        .store
        .snapshot
        .lock()
        .expect("快照锁正常")
        .as_ref()
        .expect("房间已经建好")
        .room()
        .clone()
}

/// 房主再请一个人进来：直接改存着的房间，这几条用例不看他在聊天服务器上怎么进来的。
fn seat(fixture: &Fixture, principal: PrincipalId, permissions: PrivateRoomPermissions) {
    let mut stored = fixture.store.snapshot.lock().expect("快照锁正常");
    let snapshot = stored.take().expect("房间已经建好");
    let mut room = snapshot.room().clone();
    room.invite(fixture.owner, principal, permissions)
        .expect("房主邀请");
    room.accept_invitation(principal).expect("接受邀请");
    *stored = Some(
        PrivateRoomSnapshot::new(
            snapshot.catalog().clone(),
            snapshot.instance().clone(),
            room,
        )
        .expect("快照有效"),
    );
}
