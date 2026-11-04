//! 台账里同一份授权做的一次全撤：每一条照人工撤回那一套撤，一条没撤成不挡别的，再来一次只再试没撤成的。

use agent_room_application::moderation::ReverseGrantModeration;

use super::*;

fn reverse_all(actor: AuthenticatedPrincipal) -> ReverseGrantModeration {
    ReverseGrantModeration {
        actor,
        room_catalog_id: room_id(),
        grant_id: grant_id(),
        impact_acknowledged: true,
    }
}

/// 照授权禁言、封禁同一个发言人各一次。
async fn muted_and_banned(fixture: &AgentFixture) -> (ModerationAction, ModerationAction) {
    fixture.grant(&[RoomModerationPower::Mute, RoomModerationPower::Ban]);
    let mute = fixture
        .service
        .moderate(request("@spammer:matrix.test"))
        .await
        .expect("能禁言");
    let ban = fixture
        .service
        .moderate(AgentModerate {
            power: RoomModerationPower::Ban,
            duration: None,
            ..request("@spammer:matrix.test")
        })
        .await
        .expect("能封禁");
    (mute, ban)
}

#[tokio::test]
async fn 一份授权做的一次全撤_每条照人工撤回那一套_再来一次没有可撤的() {
    let fixture = AgentFixture::new();
    let (mute, ban) = muted_and_banned(&fixture).await;

    let outcome = fixture
        .service
        .reverse_grant(reverse_all(actor(true)))
        .await
        .expect("房主能撤");

    assert_eq!(
        outcome
            .reversed
            .iter()
            .map(ModerationAction::id)
            .collect::<Vec<_>>(),
        [mute.id(), ban.id()]
    );
    assert!(
        outcome
            .reversed
            .iter()
            .all(|action| action.status() == ModerationActionStatus::Reversed)
    );
    assert!(outcome.failed.is_empty());
    assert_eq!(
        *fixture.ledger.members.reversed.lock().unwrap(),
        [(ModerationActionKind::Ban, spammer())]
    );
    assert_eq!(
        fixture.ledger.effects.reversed_rooms(),
        ["!room:matrix.test"]
    );

    let again = fixture
        .service
        .reverse_grant(reverse_all(actor(true)))
        .await
        .expect("再来一次");
    assert!(again.reversed.is_empty() && again.failed.is_empty());
}

#[tokio::test]
async fn 一条没撤成不挡别的_再来一次只再试它() {
    let fixture = AgentFixture::new();
    let (mute, ban) = muted_and_banned(&fixture).await;
    *fixture.ledger.members.failure.lock().unwrap() = Some(MatrixFailure::new(
        MatrixOperation::Unban,
        MatrixFailureKind::DependencyUnavailable,
    ));

    let outcome = fixture
        .service
        .reverse_grant(reverse_all(actor(true)))
        .await
        .expect("撤得动的照撤");
    assert_eq!(outcome.reversed[0].id(), mute.id());
    assert_eq!(
        outcome.failed,
        [(ban.id(), ModerationFailureKind::DependencyUnavailable)]
    );

    *fixture.ledger.members.failure.lock().unwrap() = None;
    let retried = fixture
        .service
        .reverse_grant(reverse_all(actor(true)))
        .await
        .expect("再来一次");
    assert_eq!(
        retried
            .reversed
            .iter()
            .map(ModerationAction::id)
            .collect::<Vec<_>>(),
        [ban.id()]
    );
}

#[tokio::test]
async fn 要最近登录_确认过影响_管得了这个房间() {
    let fixture = AgentFixture::new();
    muted_and_banned(&fixture).await;
    let refused = |request| async {
        fixture
            .service
            .reverse_grant(request)
            .await
            .unwrap_err()
            .kind()
    };

    assert_eq!(
        refused(reverse_all(actor(false))).await,
        AgentModerationFailureKind::Forbidden
    );
    assert_eq!(
        refused(ReverseGrantModeration {
            impact_acknowledged: false,
            ..reverse_all(actor(true))
        })
        .await,
        AgentModerationFailureKind::Forbidden
    );
    *fixture.ledger.authority.room_role.lock().unwrap() = ModerationRole::None;
    assert_eq!(
        refused(reverse_all(actor(true))).await,
        AgentModerationFailureKind::Forbidden
    );
    assert!(fixture.ledger.members.reversed.lock().unwrap().is_empty());
}
