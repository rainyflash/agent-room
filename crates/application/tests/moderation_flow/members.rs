//! 私人房间里台账的踢出、封禁照成员一节那一套落（`specs/public-rooms/design.md`“落到哪”）：改成员记录、
//! 在聊天服务器上踢出或封禁、把他名下的 Agent 移出；撤回和到期解除也走那里。公开大厅照旧在每个分片上做。

use super::*;

#[tokio::test]
async fn 私人房间里踢出和封禁照成员一节那一套落_不只动聊天服务器() {
    let fixture = Fixture::new();

    let kick = apply(&fixture, person_request(ModerationActionKind::Kick)).await;
    let ban = apply(&fixture, person_request(ModerationActionKind::Ban)).await;

    assert_eq!(kick.status(), ModerationActionStatus::Applied);
    assert_eq!(ban.status(), ModerationActionStatus::Applied);
    assert_eq!(
        *fixture.members.applied.lock().unwrap(),
        [
            (ModerationActionKind::Kick, target_id()),
            (ModerationActionKind::Ban, target_id()),
        ]
    );
    assert!(fixture.effects.applied_rooms().is_empty());
}

#[tokio::test]
async fn 撤回私人房间里的封禁是解封_撤回踢出是请他回来_都走成员记录() {
    let fixture = Fixture::new();
    let ban = apply(&fixture, person_request(ModerationActionKind::Ban)).await;
    let kick = apply(&fixture, person_request(ModerationActionKind::Kick)).await;

    let unbanned = reverse(&fixture, &ban).await.expect("撤回封禁");
    let invited = reverse(&fixture, &kick).await.expect("撤回踢出");

    assert_eq!(unbanned.status(), ModerationActionStatus::Reversed);
    assert_eq!(invited.status(), ModerationActionStatus::Reversed);
    assert_eq!(
        *fixture.members.reversed.lock().unwrap(),
        [
            (ModerationActionKind::Ban, target_id()),
            (ModerationActionKind::Kick, target_id()),
        ]
    );
    assert!(fixture.effects.reversed_rooms().is_empty());
}

#[tokio::test]
async fn 照成员一节做不了时不留一条动作() {
    for (refusal, kind) in [
        (
            ModerationMemberRefusal::Owner,
            ModerationFailureKind::Forbidden,
        ),
        (
            ModerationMemberRefusal::NotInRoom,
            ModerationFailureKind::Conflict,
        ),
        (
            ModerationMemberRefusal::RoomUnavailable,
            ModerationFailureKind::NotFound,
        ),
    ] {
        let fixture = Fixture::new();
        *fixture.members.refusal.lock().unwrap() = Some(refusal);

        let failure = fixture
            .service
            .apply_action(person_request(ModerationActionKind::Kick))
            .await
            .expect_err("做不了");

        assert_eq!(failure.kind(), kind, "{refusal:?}");
        assert!(fixture.repository.actions.lock().unwrap().is_empty());
        assert!(fixture.members.applied.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn 成员记录没落成时动作记成失败_照实报依赖不可用() {
    let fixture = Fixture::new();
    *fixture.members.failure.lock().unwrap() = Some(MatrixFailure::new(
        MatrixOperation::Ban,
        MatrixFailureKind::DependencyUnavailable,
    ));
    let request = person_request(ModerationActionKind::Ban);
    let action_id = request.action_id;

    let failure = fixture
        .service
        .apply_action(request)
        .await
        .expect_err("没落成");

    assert_eq!(failure.kind(), ModerationFailureKind::DependencyUnavailable);
    let stored = fixture.repository.action(action_id);
    assert_eq!(stored.status(), ModerationActionStatus::Failed);
    assert_eq!(stored.failure_code(), Some("matrix.unavailable"));
}

#[tokio::test]
async fn 私人房间里限时封禁到期时照成员一节解封() {
    let fixture = Fixture::new();
    let ban = apply(
        &fixture,
        ApplyModerationAction {
            kind: ModerationActionKind::Ban,
            expires_at: Some(time(NOW + 600_000)),
            ..action_request()
        },
    )
    .await;

    fixture.runtime.advance(600_000);
    assert_eq!(expire(&fixture).await.expired, 1);

    assert_eq!(status_of(&fixture, &ban), ModerationActionStatus::Reversed);
    assert_eq!(
        *fixture.members.reversed.lock().unwrap(),
        [(ModerationActionKind::Ban, target_id())]
    );
}

#[tokio::test]
async fn 公开大厅的踢出和封禁照旧在每个分片上做() {
    let fixture = Fixture::new();
    fixture
        .authority
        .public_lobby(&["!shard-a:matrix.test", "!shard-b:matrix.test"]);

    apply(&fixture, person_request(ModerationActionKind::Ban)).await;

    assert_eq!(
        fixture.effects.applied_rooms(),
        ["!shard-a:matrix.test", "!shard-b:matrix.test"]
    );
    assert!(fixture.members.applied.lock().unwrap().is_empty());
}

/// 房间里的管理员：对象是他时，只有房主动得了（`specs/redesign/room-roles.md` R1）。
fn seat_admin(fixture: &Fixture, owner: PrincipalId) {
    fixture.members.admins.lock().unwrap().push(target_id());
    *fixture.members.owner.lock().unwrap() = Some(owner);
}

fn someone_else() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(900))
}

#[tokio::test]
async fn 对象是管理员时管理员动不了他_不留动作也不落副作用() {
    for kind in [
        ModerationActionKind::Mute,
        ModerationActionKind::Kick,
        ModerationActionKind::Ban,
    ] {
        let fixture = Fixture::new();
        seat_admin(&fixture, someone_else());

        let failure = fixture
            .service
            .apply_action(person_request(kind))
            .await
            .expect_err("管理员动不了另一位管理员");

        assert_eq!(failure.kind(), ModerationFailureKind::OwnerOnly, "{kind:?}");
        assert!(fixture.repository.actions.lock().unwrap().is_empty());
        assert!(fixture.members.applied.lock().unwrap().is_empty());
        assert!(fixture.effects.applied_rooms().is_empty());
    }
}

#[tokio::test]
async fn 房主照旧禁言踢出封禁得了管理员() {
    for kind in [
        ModerationActionKind::Mute,
        ModerationActionKind::Kick,
        ModerationActionKind::Ban,
    ] {
        let fixture = Fixture::new();
        seat_admin(&fixture, principal_id());

        let action = apply(&fixture, person_request(kind)).await;

        assert_eq!(action.status(), ModerationActionStatus::Applied, "{kind:?}");
    }
}

#[tokio::test]
async fn 房主给管理员的禁言_不是房主的人撤不了() {
    let fixture = Fixture::new();
    seat_admin(&fixture, principal_id());
    let muted = apply(&fixture, person_request(ModerationActionKind::Mute)).await;

    // 房主换了人：现在来撤的只是管理员。
    *fixture.members.owner.lock().unwrap() = Some(someone_else());
    let failure = reverse(&fixture, &muted).await.expect_err("只有房主撤得了");

    assert_eq!(failure.kind(), ModerationFailureKind::OwnerOnly);
    assert_eq!(status_of(&fixture, &muted), ModerationActionStatus::Applied);
    assert!(fixture.effects.reversed_rooms().is_empty());
}

#[tokio::test]
async fn 隐藏管理员的消息和公开大厅里的平台管理员不归这一条管() {
    let fixture = Fixture::new();
    seat_admin(&fixture, someone_else());
    let hidden = apply(&fixture, hide_request("$event:matrix.test")).await;
    assert_eq!(hidden.status(), ModerationActionStatus::Applied);

    let fixture = Fixture::new();
    seat_admin(&fixture, someone_else());
    fixture.authority.public_lobby(&["!shard-a:matrix.test"]);
    let banned = apply(&fixture, person_request(ModerationActionKind::Ban)).await;
    assert_eq!(banned.status(), ModerationActionStatus::Applied);
}
