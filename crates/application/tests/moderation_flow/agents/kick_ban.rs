//! Agent 踢出、封禁照成员一节那一套落；解封就是撤回 Agent 做的那条封禁，人封的它解不了。

use agent_room_domain::private_rooms::PrivateRoomMembershipStatus;

use super::*;

const ALL: [RoomModerationPower; 4] = [
    RoomModerationPower::Mute,
    RoomModerationPower::Kick,
    RoomModerationPower::Ban,
    RoomModerationPower::Unban,
];

fn acting(power: RoomModerationPower) -> AgentModerate {
    AgentModerate {
        power,
        duration: None,
        reason: (power != RoomModerationPower::Unban).then_some(ModerationReason::Spam),
        ..request("@spammer-agent:matrix.test")
    }
}

impl AgentFixture {
    async fn act(
        &self,
        request: AgentModerate,
    ) -> Result<ModerationAction, AgentModerationFailureKind> {
        self.service
            .moderate(request)
            .await
            .map_err(AgentModerationFailure::kind)
    }
}

#[tokio::test]
async fn agent_踢出和封禁照成员一节那一套落_记着是它做的() {
    let fixture = AgentFixture::new();
    fixture.grant(&ALL);

    let kick = fixture.act(acting(RoomModerationPower::Kick)).await;
    let ban = fixture.act(acting(RoomModerationPower::Ban)).await;

    for (action, kind) in [
        (kick.expect("能踢出"), ModerationActionKind::Kick),
        (ban.expect("能封禁"), ModerationActionKind::Ban),
    ] {
        assert_eq!(action.kind(), kind);
        assert_eq!(action.status(), ModerationActionStatus::Applied);
        assert_eq!(action.actor_principal_id(), granter());
        assert_eq!(action.expires_at(), None);
    }
    assert_eq!(
        *fixture.ledger.members.applied.lock().unwrap(),
        [
            (ModerationActionKind::Kick, spammer()),
            (ModerationActionKind::Ban, spammer()),
        ]
    );
    assert!(fixture.ledger.effects.applied_rooms().is_empty());
    assert_eq!(fixture.store.attributions.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn 踢出时他已经不在房间里_或者对象是房主时不留一条动作() {
    let fixture = AgentFixture::new();
    fixture.grant(&ALL);
    for (refusal, kind) in [
        (
            ModerationMemberRefusal::NotInRoom,
            AgentModerationFailureKind::NotInRoom,
        ),
        (
            ModerationMemberRefusal::Owner,
            AgentModerationFailureKind::Protected,
        ),
        (
            ModerationMemberRefusal::RoomUnavailable,
            AgentModerationFailureKind::NotFound,
        ),
    ] {
        *fixture.ledger.members.refusal.lock().unwrap() = Some(refusal);
        assert_eq!(
            fixture
                .act(acting(RoomModerationPower::Kick))
                .await
                .unwrap_err(),
            kind
        );
    }
    assert!(fixture.ledger.repository.actions.lock().unwrap().is_empty());
    assert!(fixture.ledger.members.applied.lock().unwrap().is_empty());
}

#[tokio::test]
async fn agent_解封撤回它做的那条封禁_同一个请求再来交回同一条() {
    let fixture = AgentFixture::new();
    fixture.grant(&ALL);
    let ban = fixture
        .act(acting(RoomModerationPower::Ban))
        .await
        .expect("能封禁");
    let unban = acting(RoomModerationPower::Unban);

    let reversed = fixture.act(unban.clone()).await.expect("能解封");

    assert_eq!(reversed.id(), ban.id());
    assert_eq!(reversed.status(), ModerationActionStatus::Reversed);
    assert_eq!(
        *fixture.ledger.members.reversed.lock().unwrap(),
        [(ModerationActionKind::Ban, spammer())]
    );
    assert_eq!(
        *fixture.store.reversals.lock().unwrap(),
        [AgentModerationReversal {
            request_id: unban.action_id,
            attribution: AgentModerationAttribution {
                action_id: ban.id(),
                agent_id: agent_id(),
                grant_id: grant_id(),
                note: unban.note.clone(),
            },
        }]
    );

    let replayed = fixture.act(unban).await.expect("重放");
    assert_eq!(replayed.id(), ban.id());
    assert_eq!(fixture.ledger.members.reversed.lock().unwrap().len(), 1);

    // 已经解开了，换一个请求再解是没封着。
    assert_eq!(
        fixture
            .act(acting(RoomModerationPower::Unban))
            .await
            .unwrap_err(),
        AgentModerationFailureKind::NotBanned
    );

    let ledger = fixture
        .service
        .list(ListAgentModeration {
            actor: actor(false),
            room_catalog_id: room_id(),
        })
        .await
        .expect("房主和管理员看得到");
    assert_eq!(ledger.reversals[0].attribution.action_id, ban.id());
}

#[tokio::test]
async fn 人封的_agent_解不了() {
    let fixture = AgentFixture::new();
    fixture.grant(&ALL);
    let ban = apply(&fixture.ledger, person_request(ModerationActionKind::Ban)).await;
    assert_eq!(ban.status(), ModerationActionStatus::Applied);
    assert_eq!(
        fixture
            .act(acting(RoomModerationPower::Unban))
            .await
            .unwrap_err(),
        AgentModerationFailureKind::BannedByPerson
    );

    // 只在成员一节里封着（没走台账）也是人封的。
    let fixture = AgentFixture::new();
    fixture.grant(&ALL);
    {
        let mut snapshot = fixture.rooms.0.lock().unwrap();
        let mut room = snapshot.room().clone();
        room.ban_member(principal_id(), spammer())
            .expect("房主能封");
        *snapshot = PrivateRoomSnapshot::new(
            snapshot.catalog().clone(),
            snapshot.instance().clone(),
            room,
        )
        .expect("快照有效");
        assert_eq!(
            snapshot.room().member(spammer()).unwrap().status(),
            PrivateRoomMembershipStatus::Banned
        );
    }
    assert_eq!(
        fixture
            .act(acting(RoomModerationPower::Unban))
            .await
            .unwrap_err(),
        AgentModerationFailureKind::BannedByPerson
    );
    assert!(fixture.ledger.members.reversed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 解封时聊天服务器没解开_封禁照旧生效_不记是它撤的() {
    let fixture = AgentFixture::new();
    fixture.grant(&ALL);
    let ban = fixture
        .act(acting(RoomModerationPower::Ban))
        .await
        .expect("能封禁");
    *fixture.ledger.members.failure.lock().unwrap() = Some(MatrixFailure::new(
        MatrixOperation::Unban,
        MatrixFailureKind::DependencyUnavailable,
    ));

    assert_eq!(
        fixture
            .act(acting(RoomModerationPower::Unban))
            .await
            .unwrap_err(),
        AgentModerationFailureKind::DependencyUnavailable
    );
    let stored = fixture
        .ledger
        .repository
        .actions
        .lock()
        .unwrap()
        .iter()
        .find(|action| action.id() == ban.id())
        .cloned()
        .unwrap();
    assert_eq!(stored.status(), ModerationActionStatus::Applied);
    assert!(fixture.store.reversals.lock().unwrap().is_empty());
}
