//! 父房间的禁言在子房间里一起算（`specs/redesign/sub-rooms.md` 的“禁言”）：落的时候连他在里面的
//! 子房间一起落，结束的时候一起放开；哪一间自己还压着他，那一间不放。结束子房间自己的禁言时，
//! 父房间的禁言也算。

use super::*;

const SUB_A: &str = "!sub-a:matrix.test";
const SUB_B: &str = "!sub-b:matrix.test";
const PARENT: &str = "!room:matrix.test";

fn sub_catalog(value: u128) -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(value))
}

/// 被管的人在这间房下面的两间子房间里。
fn with_sub_rooms() -> Fixture {
    let fixture = Fixture::new();
    *fixture.authority.sub_rooms.lock().expect("子房间锁可用") = vec![
        (sub_catalog(31), matrix_room(SUB_A)),
        (sub_catalog(32), matrix_room(SUB_B)),
    ];
    fixture
}

/// 在这间子房间里禁言他，不限时。
fn mute_in(catalog: RoomCatalogId) -> ApplyModerationAction {
    ApplyModerationAction {
        room_catalog_id: catalog,
        ..mute(2, None)
    }
}

/// 从 `from` 起落下、撤掉的各是哪些房间。
fn since(
    fixture: &Fixture,
    applied_from: usize,
    reversed_from: usize,
) -> (Vec<String>, Vec<String>) {
    (
        fixture.effects.applied_rooms()[applied_from..].to_vec(),
        fixture.effects.reversed_rooms()[reversed_from..].to_vec(),
    )
}

#[tokio::test]
async fn 禁言连他在里面的子房间一起落_撤回时一起放开() {
    let fixture = with_sub_rooms();

    let muted = apply(&fixture, mute(2, None)).await;
    assert_eq!(muted.status(), ModerationActionStatus::Applied);
    assert_eq!(fixture.effects.applied_rooms(), [PARENT, SUB_A, SUB_B]);
    assert!(
        all_carry_agents(&fixture.effects.applied),
        "子房间里也带着他名下的 Agent，一起压下去"
    );

    reverse(&fixture, &muted).await.expect("撤回应成功");
    assert_eq!(fixture.effects.reversed_rooms(), [PARENT, SUB_A, SUB_B]);
}

#[tokio::test]
async fn 公开大厅的禁言不找子房间() {
    let fixture = with_sub_rooms();
    fixture.authority.public_lobby(&["!busy:matrix.test"]);
    *fixture.mutes.unreadable.lock().expect("读取锁可用") = true;

    let muted = apply(&fixture, mute(2, None)).await;

    assert_eq!(muted.status(), ModerationActionStatus::Applied);
    assert_eq!(fixture.effects.applied_rooms(), ["!busy:matrix.test"]);
}

#[tokio::test]
async fn 子房间读不出来时这条禁言记成没落成_不只落父房间() {
    let fixture = with_sub_rooms();
    *fixture.mutes.unreadable.lock().expect("读取锁可用") = true;
    let request = mute(2, None);
    let action_id = request.action_id;

    let failure = fixture
        .service
        .apply_action(request)
        .await
        .expect_err("读不出他在哪些子房间里，不能当成落下了");

    assert_eq!(failure.kind(), ModerationFailureKind::DependencyUnavailable);
    assert!(fixture.effects.applied_rooms().is_empty());
    assert_eq!(
        fixture.repository.action(action_id).status(),
        ModerationActionStatus::Failed
    );
}

#[tokio::test]
async fn 到期解除时子房间一起放开_这一间自己还压着他的不放_照那一条再落一次() {
    let fixture = with_sub_rooms();
    let short = apply(&fixture, mute(2, Some(HOUR))).await;
    let own = apply(&fixture, mute_in(sub_catalog(31))).await;
    let applied = fixture.effects.applied_rooms().len();

    fixture.runtime.advance(HOUR);
    assert_eq!(expire(&fixture).await.expired, 1);

    let (applied, reversed) = since(&fixture, applied, 0);
    assert_eq!(reversed, [PARENT, SUB_B], "第一间自己还禁着他，不放");
    assert_eq!(applied, [SUB_A], "照它自己的那条再落一次，确保禁着");
    assert_eq!(
        status_of(&fixture, &short),
        ModerationActionStatus::Reversed
    );
    assert_eq!(status_of(&fixture, &own), ModerationActionStatus::Applied);
}

#[tokio::test]
async fn 这一间自己的禁言到期了还没解除的不算压着_跟着父房间一起放开() {
    let fixture = with_sub_rooms();
    let parent = apply(&fixture, mute(2, None)).await;
    // 这一间自己的那条一个小时前就到期了，后台还没来得及记成解除。
    let mut own = ModerationAction::reserve(
        ModerationActionId::from_uuid(Uuid::now_v7()),
        None,
        principal_id(),
        sub_catalog(31),
        ModerationActionKind::Mute,
        ModerationTarget::new(ModerationTargetKind::Principal, target_id().to_string())
            .expect("目标有效"),
        ModerationReason::Spam,
        time(NOW - 2 * HOUR),
        Some(time(NOW - HOUR)),
    )
    .expect("禁言有效");
    own.mark_applied().expect("可应用");
    fixture
        .repository
        .actions
        .lock()
        .expect("动作锁可用")
        .push(own);

    reverse(&fixture, &parent).await.expect("撤回应成功");

    assert_eq!(fixture.effects.reversed_rooms(), [PARENT, SUB_A, SUB_B]);
}

#[tokio::test]
async fn 结束子房间自己的禁言时父房间的还在_不放人_在这一间确保禁着() {
    let fixture = with_sub_rooms();
    let parent = apply(&fixture, mute(2, None)).await;
    let own = apply(&fixture, mute_in(sub_catalog(31))).await;
    let applied = fixture.effects.applied_rooms().len();

    let reversed = reverse(&fixture, &own).await.expect("撤回应成功");

    assert_eq!(reversed.status(), ModerationActionStatus::Reversed);
    let (applied, lifted) = since(&fixture, applied, 0);
    assert!(lifted.is_empty(), "父房间的禁言还在，这一间也不能放");
    assert_eq!(applied, [SUB_A]);
    assert_eq!(
        status_of(&fixture, &parent),
        ModerationActionStatus::Applied
    );

    // 父房间的也撤了：这一间才放开。
    reverse(&fixture, &parent).await.expect("撤回应成功");
    assert_eq!(fixture.effects.reversed_rooms(), [PARENT, SUB_A, SUB_B]);
}

#[tokio::test]
async fn 只在子房间里的禁言只落这一间_撤回也只动这一间() {
    let fixture = with_sub_rooms();

    let own = apply(&fixture, mute_in(sub_catalog(32))).await;
    assert_eq!(fixture.effects.applied_rooms(), [SUB_B]);

    reverse(&fixture, &own).await.expect("撤回应成功");
    assert_eq!(fixture.effects.reversed_rooms(), [SUB_B]);
}

#[tokio::test]
async fn 还有别的禁言在生效时确保禁着_连子房间一起() {
    let fixture = with_sub_rooms();
    let first = apply(&fixture, mute(2, None)).await;
    apply(&fixture, mute(2, None)).await;
    let applied = fixture.effects.applied_rooms().len();

    reverse(&fixture, &first).await.expect("撤回应成功");

    let (applied, lifted) = since(&fixture, applied, 0);
    assert!(lifted.is_empty());
    assert_eq!(applied, [PARENT, SUB_A, SUB_B]);
}

#[tokio::test]
async fn 动完再看一眼他又进了一间子房间_这次不记_下一轮连那一间一起放开() {
    let fixture = with_sub_rooms();
    let short = apply(&fixture, mute(2, Some(HOUR))).await;
    fixture.runtime.advance(HOUR);
    let sub_rooms = fixture.authority.sub_rooms.clone();
    fixture.mutes.between_reads(move || {
        sub_rooms
            .lock()
            .expect("子房间锁可用")
            .push((sub_catalog(33), matrix_room("!sub-c:matrix.test")));
    });

    assert_eq!(expire(&fixture).await.expired, 0);
    assert_eq!(status_of(&fixture, &short), ModerationActionStatus::Applied);

    fixture.runtime.advance(20 * SECOND);
    assert_eq!(expire(&fixture).await.expired, 1);
    assert_eq!(
        fixture.effects.reversed_rooms()[3..],
        [PARENT, SUB_A, SUB_B, "!sub-c:matrix.test"]
    );
}

#[tokio::test]
async fn 撤回的当口这一间自己落了一条禁言_动完再看一眼_把这一间压回去() {
    let fixture = with_sub_rooms();
    let parent = apply(&fixture, mute(2, None)).await;
    let applied = fixture.effects.applied_rooms().len();
    // 撤父房间的禁言时已经把这一间放开了；这当口有人在这一间里单独禁了他，它落的那一下被盖掉了。
    let landing = ModerationAction::reserve(
        ModerationActionId::from_uuid(Uuid::now_v7()),
        None,
        principal_id(),
        sub_catalog(31),
        ModerationActionKind::Mute,
        ModerationTarget::new(ModerationTargetKind::Principal, target_id().to_string())
            .expect("目标有效"),
        ModerationReason::Spam,
        fixture.runtime.now(),
        None,
    )
    .expect("禁言有效");
    let repository = fixture.repository.clone();
    fixture.mutes.between_reads(move || {
        repository.actions.lock().expect("动作锁可用").push(landing);
    });

    reverse(&fixture, &parent).await.expect("撤回应成功");

    let (applied, lifted) = since(&fixture, applied, 0);
    assert_eq!(
        lifted,
        [PARENT, SUB_A, SUB_B, PARENT, SUB_B],
        "再动一次时这一间不放了"
    );
    assert_eq!(applied, [SUB_A], "照这一间正在落的那条压回去");
}
