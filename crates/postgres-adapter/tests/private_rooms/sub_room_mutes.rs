//! 父房间的禁言在子房间里一起算（`specs/redesign/sub-rooms.md` 的“禁言”，2c-3）。这里查存储读的
//! 那一半：一个人的禁言情况带上他在里面的子房间（他自己不在、只有他名下的 Agent 成员在的也算，
//! 2f-2b-2）、子房间里连父房间的禁言一起读、他此刻是不是被禁言着、禁言锁一家子共用一把。

use agent_room_application::ports::{
    ModerationMuteLedger, ModerationMuteLock, ModerationMuteStanding, ModerationRepository,
    StandingModerationSource,
};
use agent_room_domain::{
    ids::{AuditEventId, ModerationActionId},
    join_codes::PrivateRoomAgentJoinedVia,
    moderation::{
        ModerationAction, ModerationActionKind, ModerationAuditEvent, ModerationAuditOutcome,
        ModerationReason, ModerationTarget, ModerationTargetKind,
    },
};

use super::{
    agent_members::{admit_agent, remove},
    membership::{change, enter},
    sync::open,
    *,
};

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 读一个人的禁言_带上他在里面的子房间和那一间自己的禁言_子房间里连父房间的一起读() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-mutes-read-owner").await;
    let member = seed_principal(pool, "sub-room-mutes-read-member").await;
    let other = seed_principal(pool, "sub-room-mutes-read-other").await;
    let parent = create_room(&repositories, owner).await;
    for person in [member, other] {
        admit(&repositories, parent, owner, person).await;
    }
    let alpha = open(&repositories, parent, "Alpha", 10).await;
    let beta = open(&repositories, parent, "Beta", 11).await;
    let gamma = open(&repositories, parent, "Gamma", 12).await;
    for (sub_room, person, at) in [(alpha, member, 20), (beta, member, 21), (gamma, other, 22)] {
        assert!(enter(&repositories, parent, sub_room, person, time(at)).await);
    }
    let shouted = ModerationTarget::new(
        ModerationTargetKind::Principal,
        member.as_uuid().simple().to_string().to_uppercase(),
    )
    .expect("大写、不带横线的 UUID 也是有效目标");
    let above = applied(&repositories, owner, parent, &person(member), 30, None).await;
    let own = applied(&repositories, owner, alpha, &person(member), 31, None).await;
    // 别人在同一间里的禁言不算他的；另一间里他有一条正在落的，引用的写法还不一样。
    applied(&repositories, owner, alpha, &person(other), 32, None).await;
    let landing = reserved(&repositories, owner, beta, &shouted, 33, None).await;

    let listed = standing_in(&repositories, parent, &person(member)).await;
    assert_eq!(
        action_ids(&listed.mutes),
        [above.id()],
        "父房间自己的禁言里不混进子房间的"
    );
    assert_eq!(
        listed
            .sub_rooms
            .iter()
            .map(|sub_room| (
                sub_room.matrix_room_id.as_str().to_owned(),
                action_ids(&sub_room.mutes)
            ))
            .collect::<Vec<_>>(),
        [
            (matrix_room(&repositories, alpha).await, vec![own.id()]),
            (matrix_room(&repositories, beta).await, vec![landing.id()]),
        ],
        "只有他在里面的那两间，先建的在前，各带着那一间自己的禁言"
    );

    let inside = standing_in(&repositories, alpha, &person(member)).await;
    assert_eq!(
        action_ids(&inside.mutes),
        [above.id(), own.id()],
        "子房间里连父房间的一起算"
    );
    assert!(inside.sub_rooms.is_empty());
    assert_eq!(
        inside
            .matrix_room_ids
            .iter()
            .map(|room| room.as_str().to_owned())
            .collect::<Vec<_>>(),
        [matrix_room(&repositories, alpha).await]
    );
    assert!(inside.may_speak);

    // 房主在每一间里；对象不是人的没有子房间可找。
    let everywhere = standing_in(&repositories, parent, &person(owner)).await;
    assert_eq!(everywhere.sub_rooms.len(), 3);
    assert!(
        everywhere
            .sub_rooms
            .iter()
            .all(|sub_room| sub_room.mutes.is_empty())
    );
    let event = ModerationTarget::new(ModerationTargetKind::Event, "$event:matrix.test")
        .expect("事件目标有效");
    assert!(
        standing_in(&repositories, parent, &event)
            .await
            .sub_rooms
            .is_empty()
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 他已经出去的子房间和归档了的子房间不在里面() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-mutes-gone-owner").await;
    let member = seed_principal(pool, "sub-room-mutes-gone-member").await;
    let parent = create_room(&repositories, owner).await;
    let closing = create_room(&repositories, owner).await;
    for room in [parent, closing] {
        admit(&repositories, room, owner, member).await;
    }
    let alpha = open(&repositories, parent, "Alpha", 10).await;
    let beta = open(&repositories, parent, "Beta", 11).await;
    // 要归档的那间房归档在 10 这一刻：它下面的子房间得建得更早。
    let old = open(&repositories, closing, "Old", 5).await;
    for (above, sub_room, at) in [(parent, alpha, 20), (parent, beta, 21), (closing, old, 6)] {
        assert!(enter(&repositories, above, sub_room, member, time(at)).await);
    }

    change(&repositories, beta, time(30), |room| {
        room.leave(member).expect("能退出这一间");
    })
    .await;
    let listed = standing_in(&repositories, parent, &person(member)).await;
    assert_eq!(
        listed
            .sub_rooms
            .iter()
            .map(|sub_room| sub_room.matrix_room_id.as_str().to_owned())
            .collect::<Vec<_>>(),
        [matrix_room(&repositories, alpha).await],
        "退出了的那一间不用动"
    );

    archive(&repositories, closing, owner).await;
    let closed = standing_in(&repositories, closing, &person(member)).await;
    assert!(closed.matrix_room_ids.is_empty());
    assert!(
        closed.sub_rooms.is_empty(),
        "归档了的子房间一动发言级别就等于解开了，不能给出去"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 他自己不在_他名下的_agent_成员在的子房间也算进去_移出了的_收回了的和别人的不算() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-mutes-agents-owner").await;
    let outsider = seed_principal(pool, "sub-room-mutes-agents-outsider").await;
    let stranger = seed_principal(pool, "sub-room-mutes-agents-stranger").await;
    let agent = seed_agent(pool, outsider).await;
    let strangers = seed_agent(pool, stranger).await;
    let parent = create_room(&repositories, owner).await;
    let alpha = open(&repositories, parent, "Alpha", 10).await;
    let beta = open(&repositories, parent, "Beta", 11).await;
    let gamma = open(&repositories, parent, "Gamma", 12).await;
    for each in [agent, strangers] {
        admit_agent(
            &repositories,
            parent,
            each,
            PrivateRoomAgentJoinedVia::Code,
            20,
        )
        .await;
    }
    for (sub_room, each, at) in [
        (alpha, agent, 30),
        (beta, agent, 31),
        (gamma, strangers, 32),
    ] {
        admit_agent(
            &repositories,
            sub_room,
            each,
            PrivateRoomAgentJoinedVia::Knock,
            at,
        )
        .await;
    }
    let own = applied(&repositories, owner, alpha, &person(outsider), 40, None).await;
    let rooms_of = |standing: ModerationMuteStanding| {
        standing
            .sub_rooms
            .into_iter()
            .map(|sub_room| {
                (
                    sub_room.matrix_room_id.as_str().to_owned(),
                    action_ids(&sub_room.mutes),
                )
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(
        rooms_of(standing_in(&repositories, parent, &person(outsider)).await),
        [
            (matrix_room(&repositories, alpha).await, vec![own.id()]),
            (matrix_room(&repositories, beta).await, Vec::new()),
        ],
        "他不在这几间里，他的 Agent 在：禁言落的时候要连它一起压"
    );

    assert!(remove(&repositories, beta, agent, 50).await);
    assert_eq!(
        rooms_of(standing_in(&repositories, parent, &person(outsider)).await),
        [(matrix_room(&repositories, alpha).await, vec![own.id()])],
        "在那一间里移出了的不算"
    );

    // 还在用的 Agent 总得有一个主人（数据库的触发器在提交时查），所以收回他的同时交给别人。
    let mut transfer = pool.begin().await.expect("转交 Agent 的事务应启动");
    sqlx::query(
        "UPDATE agent_room.agent_ownership SET revoked_at = clock_timestamp() WHERE agent_id = $1",
    )
    .bind(agent.as_uuid())
    .execute(&mut *transfer)
    .await
    .expect("收回 Agent");
    sqlx::query(
        r"INSERT INTO agent_room.agent_ownership (principal_id, agent_id, role, granted_by, created_at)
          VALUES ($1, $2, 'owner', $1, clock_timestamp())",
    )
    .bind(stranger.as_uuid())
    .bind(agent.as_uuid())
    .execute(&mut *transfer)
    .await
    .expect("交给别人");
    transfer.commit().await.expect("转交 Agent 应提交");
    assert!(
        standing_in(&repositories, parent, &person(outsider))
            .await
            .sub_rooms
            .is_empty(),
        "不再是他的 Agent 了"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 他此刻是不是被禁言着_子房间里连父房间的一起算_正在落的也算_到期和撤回的不算() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-mutes-now-owner").await;
    let member = seed_principal(pool, "sub-room-mutes-now-member").await;
    let other = seed_principal(pool, "sub-room-mutes-now-other").await;
    let parent = create_room(&repositories, owner).await;
    for person in [member, other] {
        admit(&repositories, parent, owner, person).await;
    }
    let alpha = open(&repositories, parent, "Alpha", 10).await;
    let beta = open(&repositories, parent, "Beta", 11).await;
    assert!(!muted(&repositories, alpha, member, 100).await);

    // 父房间里限时的一条：到期以前在每一间里都算，只算他一个人。
    let mut above = applied(
        &repositories,
        owner,
        parent,
        &person(member),
        100,
        Some(1_000),
    )
    .await;
    for room in [parent, alpha, beta] {
        assert!(muted(&repositories, room, member, 500).await);
        assert!(!muted(&repositories, room, other, 500).await);
        assert!(
            !muted(&repositories, room, member, 1_000).await,
            "到了期限就不算了，后台还没记成解除也一样"
        );
    }
    above.reverse(time(600)).expect("可撤回");
    finish(&repositories, &above, "moderation.action.reversed").await;
    assert!(!muted(&repositories, alpha, member, 700).await);

    // 只在一间子房间里正在落的一条：这一间里算，父房间和别的子房间里不算；落了五分钟还没落完的不算。
    reserved(&repositories, owner, alpha, &person(member), 2_000, None).await;
    assert!(muted(&repositories, alpha, member, 2_001).await);
    assert!(!muted(&repositories, parent, member, 2_001).await);
    assert!(!muted(&repositories, beta, member, 2_001).await);
    assert!(!muted(&repositories, alpha, member, 2_000 + 300_000).await);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 禁言锁_子房间和它的父房间是同一把_别的房间下面的不相干() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-mutes-lock-owner").await;
    let member = seed_principal(pool, "sub-room-mutes-lock-member").await;
    let other = seed_principal(pool, "sub-room-mutes-lock-other").await;
    let parent = create_room(&repositories, owner).await;
    let elsewhere = create_room(&repositories, owner).await;
    let alpha = open(&repositories, parent, "Alpha", 10).await;
    let beta = open(&repositories, parent, "Beta", 11).await;
    let gamma = open(&repositories, elsewhere, "Gamma", 12).await;

    let held = lock(&repositories, parent, member)
        .await
        .expect("没人拿着时拿得到");
    for room in [alpha, beta] {
        assert!(
            lock(&repositories, room, member).await.is_none(),
            "父房间那边正在处理他，子房间这边等着"
        );
    }
    for (room, who) in [(alpha, other), (gamma, member), (elsewhere, member)] {
        lock(&repositories, room, who)
            .await
            .expect("换个人、换一家互不影响")
            .release()
            .await;
    }
    held.release().await;

    let held = lock(&repositories, alpha, member)
        .await
        .expect("放掉以后子房间这边拿得到");
    assert!(lock(&repositories, parent, member).await.is_none());
    assert!(lock(&repositories, beta, member).await.is_none());
    held.release().await;

    database.close().await;
}

fn person(principal: PrincipalId) -> ModerationTarget {
    ModerationTarget::new(ModerationTargetKind::Principal, principal.to_string())
        .expect("人是有效目标")
}

fn action_ids(actions: &[ModerationAction]) -> Vec<ModerationActionId> {
    actions.iter().map(ModerationAction::id).collect()
}

async fn matrix_room(repositories: &PostgresRepositories, catalog: RoomCatalogId) -> String {
    load(repositories, catalog)
        .await
        .instance()
        .matrix_room_id()
        .as_str()
        .to_owned()
}

async fn standing_in(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
    target: &ModerationTarget,
) -> ModerationMuteStanding {
    ModerationMuteLedger::mute_standing(repositories, catalog, target)
        .await
        .expect("禁言情况应可读取")
        .expect("有这间房")
}

async fn muted(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
    principal: PrincipalId,
    at: i64,
) -> bool {
    StandingModerationSource::person_muted(repositories, catalog, principal, time(at))
        .await
        .expect("可读")
}

async fn lock(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
    principal: PrincipalId,
) -> Option<Box<dyn ModerationMuteLock>> {
    ModerationMuteLedger::try_lock_mutes(repositories, catalog, &person(principal))
        .await
        .expect("禁言锁应可拿")
}

/// 预留一条禁言：记成正在落，还没生效。
async fn reserved(
    repositories: &PostgresRepositories,
    actor: PrincipalId,
    catalog: RoomCatalogId,
    target: &ModerationTarget,
    starts_at: i64,
    expires_at: Option<i64>,
) -> ModerationAction {
    let action = ModerationAction::reserve(
        ModerationActionId::from_uuid(Uuid::now_v7()),
        None,
        actor,
        catalog,
        ModerationActionKind::Mute,
        target.clone(),
        ModerationReason::Spam,
        time(starts_at),
        expires_at.map(time),
    )
    .expect("禁言有效");
    ModerationRepository::reserve_action(
        repositories,
        &action,
        &audit(&action, "moderation.action.requested"),
    )
    .await
    .expect("预留应成功");
    action
}

/// 一条已经落下的禁言。
async fn applied(
    repositories: &PostgresRepositories,
    actor: PrincipalId,
    catalog: RoomCatalogId,
    target: &ModerationTarget,
    starts_at: i64,
    expires_at: Option<i64>,
) -> ModerationAction {
    let mut action = reserved(repositories, actor, catalog, target, starts_at, expires_at).await;
    action.mark_applied().expect("可应用");
    finish(repositories, &action, "moderation.action.applied").await;
    action
}

/// 把动作改成的终态落库。
async fn finish(repositories: &PostgresRepositories, action: &ModerationAction, code: &str) {
    ModerationRepository::finalize_action(repositories, action, &audit(action, code))
        .await
        .expect("终态应提交");
}

fn audit(action: &ModerationAction, code: &str) -> ModerationAuditEvent {
    ModerationAuditEvent::new(
        AuditEventId::from_uuid(Uuid::now_v7()),
        action.reversed_at().unwrap_or(action.starts_at()),
        action.actor_principal_id(),
        code,
        action.target().clone(),
        ModerationAuditOutcome::Allowed,
        Some(action.reason()),
        AuditEventId::from_uuid(action.id().as_uuid()),
        Some(action.room_catalog_id()),
    )
    .expect("动作审计有效")
}
