//! 子房间第 2f-2a 步（`specs/redesign/sub-rooms.md` 的“加入和退出”）：凭口令、敲门进了父房间的 Agent
//! 成员拿房间号进子房间。没有门的直接进；要批准的敲门、等管得了这一间的人放行；只能请进来的和藏起来的
//! 不收敲门，由管理者按名字放行；不是父房间里的 Agent 成员的当找不到。
//!
//! 夹具：父房间（房主、能发言的成员、一个普通成员 `manager`）下面一间子房间，`manager` 是这一间的
//! 子房间管理员，能发言的成员在里面但管不了。

use agent_room_application::private_rooms::AgentAccessFailure;
use agent_room_domain::{ids::PrincipalId, sub_rooms::SubRoomDoor};

use super::*;

pub(super) fn parent_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(77))
}

/// 父房间里的普通成员，这间子房间的子房间管理员。
fn manager() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(4))
}

fn join(room: &mut PrivateRoom, person: PrincipalId, capabilities: &[PrivateRoomCapability]) {
    room.invite(
        owner(),
        person,
        PrivateRoomPermissions::from_capabilities(capabilities.iter().copied()).unwrap(),
    )
    .unwrap();
    room.accept_invitation(person).unwrap();
}

/// 父房间和下面一间子房间，`access`、`hidden` 是这一间的门。
pub(super) fn family(access: RoomJoinMode, hidden: bool) -> Fixture {
    use PrivateRoomCapability::{Manage, Speak, View};
    let mut parent = with_speaker(PrivateRoom::create(parent_id(), owner()));
    join(&mut parent, manager(), &[View, Speak]);
    let mut sub_room = active_room();
    join(&mut sub_room, manager(), &[View, Speak, Manage]);
    family_of(parent, sub_room, access, hidden)
}

fn family_of(
    parent: PrivateRoom,
    sub_room: PrivateRoom,
    access: RoomJoinMode,
    hidden: bool,
) -> Fixture {
    let door = SubRoomDoor::new(access, hidden).expect("门有效");
    let link = SubRoomLink {
        catalog_id: catalog_id(),
        parent_catalog_id: parent_id(),
        name_key: "release".to_owned(),
        access: door.access(),
        hidden: door.hidden(),
        door_opened_at: None,
        end_to_end_encrypted: true,
        created_by: owner(),
        created_by_agent: None,
        created_at: time(NOW - HOUR),
        archived: None,
    };
    let parent = snapshot_in(
        parent_id(),
        MatrixRoomReference::new("!parent:matrix.test".to_owned()).unwrap(),
        parent,
    );
    Fixture::with_rooms(
        vec![parent, snapshot(sub_room).under(parent_id())],
        Some(link),
    )
}

impl Fixture {
    /// 这个 Agent 凭口令进了父房间。
    async fn in_parent(&self) {
        self.store
            .admit_agent(
                parent_id(),
                agent_id(),
                PrivateRoomPermissions::AGENT_MEMBER,
                PrivateRoomAgentJoinedVia::Code,
                time(NOW - HOUR),
            )
            .await
            .unwrap();
    }

    async fn knock(&self) -> Result<KnockResult, AgentAccessFailureKind> {
        self.service
            .knock(knock_request())
            .await
            .map_err(AgentAccessFailure::kind)
    }

    async fn admit(&self, actor: PrincipalId) -> Result<(), AgentAccessFailureKind> {
        self.service
            .admit_knock(answer(actor))
            .await
            .map(|admitted| assert_eq!(admitted.room.catalog_id, catalog_id()))
            .map_err(AgentAccessFailure::kind)
    }
}

fn admitted(result: Result<KnockResult, AgentAccessFailureKind>) -> RoomCatalogId {
    match result {
        Ok(KnockResult::Member(room)) => room.catalog_id,
        other => panic!("应该直接进来：{other:?}"),
    }
}

#[tokio::test]
async fn 没有门的_父房间里的_agent_成员敲门就进来_记成敲门进来的_不碰聊天服务器() {
    let fixture = family(RoomJoinMode::Open, false);
    assert_eq!(
        fixture.service.knockable(catalog_id()).await.unwrap(),
        KnockEntry::SubRoom,
        "子房间不占起名和敲门的名额"
    );
    fixture.in_parent().await;

    assert_eq!(admitted(fixture.knock().await), catalog_id());
    assert_eq!(
        fixture.store.status_in(catalog_id(), agent_id()),
        Some(PrivateRoomAgentMemberStatus::Joined)
    );
    assert_eq!(
        fixture.store.joined_via(agent_id()),
        Some(PrivateRoomAgentJoinedVia::Knock),
        "直接放进来的照样记成敲门进来的"
    );
    assert_eq!(fixture.store.knock_status(agent_id()), None, "没有敲门记录");
    assert_eq!(
        admitted(fixture.knock().await),
        catalog_id(),
        "再敲还是进来"
    );
    assert!(
        fixture.matrix.events().is_empty(),
        "进聊天服务器上的房间是进门那一步的事"
    );
}

#[tokio::test]
async fn 不是父房间里的_agent_成员敲门当找不到_在父房间里被移出的也一样() {
    let fixture = family(RoomJoinMode::Open, false);
    assert_eq!(
        fixture.knock().await.unwrap_err(),
        AgentAccessFailureKind::NotFound
    );

    fixture.in_parent().await;
    fixture
        .store
        .remove_agent(parent_id(), agent_id(), time(NOW - 1_000))
        .await
        .unwrap();
    assert_eq!(
        fixture.knock().await.unwrap_err(),
        AgentAccessFailureKind::NotFound
    );
    assert_eq!(fixture.store.status_in(catalog_id(), agent_id()), None);
    assert_eq!(
        fixture.admit(owner()).await.unwrap_err(),
        AgentAccessFailureKind::NotFound,
        "管理者也放不进来"
    );
}

#[tokio::test]
async fn 要批准的敲门等放行_管得了这一间的人看得到_放行以后记成_agent_成员() {
    let fixture = family(RoomJoinMode::Request, false);
    fixture.in_parent().await;

    assert!(matches!(fixture.knock().await, Ok(KnockResult::Waiting(_))));
    assert_eq!(fixture.store.status_in(catalog_id(), agent_id()), None);
    for actor in [owner(), manager()] {
        assert_eq!(
            waiting_agents(&fixture, actor).await,
            [agent_id()],
            "父房间的房主、这一间的管理员"
        );
    }
    for (actor, kind) in [
        (speaker(), AgentAccessFailureKind::Forbidden),
        (outsider(), AgentAccessFailureKind::NotFound),
    ] {
        assert_eq!(
            fixture
                .service
                .waiting_knocks(inspect(actor))
                .await
                .unwrap_err()
                .kind(),
            kind,
            "在里面管不了的说没权限，外面的说找不到"
        );
    }
    assert_eq!(
        fixture.admit(speaker()).await.unwrap_err(),
        AgentAccessFailureKind::Forbidden
    );

    fixture.admit(manager()).await.expect("这一间的管理员放行");
    assert_eq!(
        fixture.store.status_in(catalog_id(), agent_id()),
        Some(PrivateRoomAgentMemberStatus::Joined)
    );
    fixture
        .service
        .complete_knock(answer(manager()))
        .await
        .expect("进去了");
    assert_eq!(
        fixture.store.knock_status(agent_id()),
        Some(PrivateRoomAgentKnockStatus::Admitted)
    );
}

#[tokio::test]
async fn 只能请进来的和藏起来的不收敲门_管理者按名字放行_不用先有敲门() {
    for (access, hidden) in [
        (RoomJoinMode::InviteOnly, false),
        (RoomJoinMode::InviteOnly, true),
        (RoomJoinMode::Request, true),
    ] {
        let fixture = family(access, hidden);
        fixture.in_parent().await;
        assert_eq!(
            fixture.knock().await.unwrap_err(),
            AgentAccessFailureKind::InviteOnly,
            "{access:?} 藏起来 {hidden}"
        );
        assert_eq!(fixture.store.knock_status(agent_id()), None, "没记下敲门");

        fixture
            .admit(owner())
            .await
            .expect("父房间的房主按名字放行");
        assert_eq!(
            fixture.store.status_in(catalog_id(), agent_id()),
            Some(PrivateRoomAgentMemberStatus::Joined)
        );
        assert_eq!(
            admitted(fixture.knock().await),
            catalog_id(),
            "放进来以后敲门就进"
        );
    }
}

#[tokio::test]
async fn 没有门的_被这一间的管理员移出过的要敲门等放行() {
    let fixture = family(RoomJoinMode::Open, false);
    fixture.in_parent().await;
    admitted(fixture.knock().await);

    fixture
        .service
        .remove_agent(RemoveAgentMember {
            actor: principal_actor(manager()).into(),
            catalog_id: catalog_id(),
            agent_id: agent_id(),
        })
        .await
        .expect("这一间的管理员能移出");
    assert_eq!(fixture.matrix.events(), ["speaking:false", "kick"]);
    assert_eq!(
        fixture.store.status_in(catalog_id(), agent_id()),
        Some(PrivateRoomAgentMemberStatus::Removed)
    );
    assert_eq!(
        fixture.store.status_in(parent_id(), agent_id()),
        Some(PrivateRoomAgentMemberStatus::Joined),
        "父房间里的不动"
    );

    assert!(matches!(fixture.knock().await, Ok(KnockResult::Waiting(_))));
    assert_eq!(waiting_agents(&fixture, manager()).await, [agent_id()]);
}

#[tokio::test]
async fn 父房间里移出_agent_成员时_先从它在的子房间里踢出去_子房间的记录跟着移出() {
    let fixture = family(RoomJoinMode::Open, false);
    fixture.in_parent().await;
    admitted(fixture.knock().await);

    fixture
        .service
        .remove_agent(RemoveAgentMember {
            actor: principal_actor(owner()).into(),
            catalog_id: parent_id(),
            agent_id: agent_id(),
        })
        .await
        .expect("父房间的房主能移出");
    assert_eq!(
        fixture.matrix.events(),
        ["membership", "kick", "speaking:false", "kick"],
        "先踢子房间里的，再踢父房间里的"
    );
    for room in [parent_id(), catalog_id()] {
        assert_eq!(
            fixture.store.status_in(room, agent_id()),
            Some(PrivateRoomAgentMemberStatus::Removed)
        );
    }
    assert_eq!(
        *fixture.store.queued_kicks.lock().unwrap(),
        [(catalog_id(), agent_id())],
        "存储照旧排上，后台那一遍看到它已经出去了就不再踢"
    );
    assert_eq!(
        fixture.knock().await.unwrap_err(),
        AgentAccessFailureKind::NotFound,
        "不是父房间里的 Agent 成员了"
    );
}

#[tokio::test]
async fn 这一间归档了_敲门当找不到_管理者也放行不了() {
    use PrivateRoomCapability::{Manage, Speak, View};
    let mut parent = with_speaker(PrivateRoom::create(parent_id(), owner()));
    join(&mut parent, manager(), &[View, Speak]);
    let mut sub_room = active_room();
    join(&mut sub_room, manager(), &[View, Speak, Manage]);
    sub_room.archive(owner()).unwrap();
    let fixture = family_of(parent, sub_room, RoomJoinMode::Open, false);
    fixture.in_parent().await;

    assert_eq!(
        fixture.knock().await.unwrap_err(),
        AgentAccessFailureKind::NotFound
    );
    assert_eq!(
        fixture.admit(owner()).await.unwrap_err(),
        AgentAccessFailureKind::Conflict
    );
    assert_eq!(fixture.store.status_in(catalog_id(), agent_id()), None);
}
