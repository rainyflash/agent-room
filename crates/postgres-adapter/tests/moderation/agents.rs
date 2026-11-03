//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：认人（人的账号是他，Agent 的
//! 账号认成它的主人，网络 Agent 标出来）；Agent 做的动作和台账同一个事务记下，重放不再记；授权删了，理由
//! 跟着删、动作照旧留着；下载的数据里带着他的 Agent 写下的理由。Agent 解封撤回的封禁也一样：和撤回同一个
//! 事务记下，按请求 ID 找得回来，授权删了理由跟着删。

use agent_room_application::{
    persistence::RepositoryErrorKind,
    ports::{
        AccountDeletionRepository, AgentModerationAttribution, AgentModerationReversal,
        AgentModerationStore, ModerationSubject, RoomModerationGrant, RoomModerationGrantStore,
    },
};
use agent_room_domain::{
    ids::{AgentId, RoomModerationGrantId},
    network_agents::NETWORK_AGENT_ISSUER,
    room_moderation_grants::{
        AgentModerationNote, RoomModerationPower, RoomModerationPowers, RoomModerationRules,
    },
};

use super::*;

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 认人_人的账号是他_agent_认成主人_网络_agent_标出来() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let person = seed_principal(&database.runtime, "subject-person").await;
    let person_agent = seed_agent(&database.runtime, person).await;
    let network = seed_network_principal(&database.runtime).await;
    let network_agent = seed_agent(&database.runtime, network).await;

    let subject = |user: String| {
        let repositories = &repositories;
        async move {
            repositories
                .moderation_subject(&MatrixUserId::new(user).expect("账号有效"))
                .await
                .expect("可读")
        }
    };
    let as_person = Some(ModerationSubject {
        principal_id: person,
        network_agent: false,
    });
    assert_eq!(
        subject(matrix_user(&database.runtime, person).await).await,
        as_person
    );
    assert_eq!(subject(person_agent).await, as_person);
    assert_eq!(
        subject(network_agent).await,
        Some(ModerationSubject {
            principal_id: network,
            network_agent: true,
        })
    );
    // 网络 Agent 的合成主体那个账号从不进房间，不认。
    assert_eq!(
        subject(matrix_user(&database.runtime, network).await).await,
        None
    );
    assert_eq!(subject("@nobody:matrix.test".to_owned()).await, None);
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn agent_做的动作和台账一起记下_重放不再记_删了授权理由跟着删() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "agent-grant-owner").await;
    let target = seed_principal(&database.runtime, "agent-grant-target").await;
    let agent_user = seed_agent(&database.runtime, owner).await;
    let agent_id = agent_of(&database.runtime, &agent_user).await;
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    PrivateRoomStore::create(
        &repositories,
        &private_room_snapshot(catalog_id, owner),
        time(0),
    )
    .await
    .expect("私人房间夹具应创建");
    assert!(no_grant(&repositories, catalog_id, agent_id).await);
    let grant = grant(catalog_id, agent_id, owner);
    repositories
        .replace_room_moderation_grant(&grant)
        .await
        .expect("给上");
    let found = repositories
        .active_agent_grant(catalog_id, agent_id, time(10))
        .await
        .expect("可读");
    assert_eq!(found.as_ref().map(|found| found.id), Some(grant.id));
    assert!(
        repositories
            .active_agent_grant(catalog_id, agent_id, grant.expires_at)
            .await
            .expect("可读")
            .is_none(),
        "到期了不认"
    );

    let action = mute_action(owner, catalog_id, target);
    let attribution = AgentModerationAttribution {
        action_id: action.id(),
        agent_id,
        grant_id: grant.id,
        note: AgentModerationNote::parse("连发了五条广告链接。").expect("理由有效"),
    };
    let audit = action_audit(
        &action,
        "moderation.action.requested",
        ModerationAuditOutcome::Allowed,
    );
    let reserve = || repositories.reserve_agent_action(&action, &audit, &attribution);
    assert!(matches!(
        reserve().await.expect("预留"),
        ModerationActionReservationOutcome::Reserved(_)
    ));
    assert!(matches!(
        reserve().await.expect("重放"),
        ModerationActionReservationOutcome::Existing(_)
    ));
    let mismatched = AgentModerationAttribution {
        action_id: ModerationActionId::from_uuid(Uuid::now_v7()),
        ..attribution.clone()
    };
    let error = repositories
        .reserve_agent_action(&action, &audit, &mismatched)
        .await
        .expect_err("记的不是这个动作");
    assert_eq!(error.kind(), RepositoryErrorKind::Constraint);

    let records = repositories.agent_actions(catalog_id).await.expect("可读");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].attribution, attribution);
    assert_eq!(records[0].agent_display_name, "治理测试 Agent");
    let exported = AccountDeletionRepository::export(&repositories, owner, time(20))
        .await
        .expect("导出")
        .expect("账户还在");
    assert_eq!(
        exported.data["agentModerationNotes"][0]["note"],
        "连发了五条广告链接。"
    );

    sqlx::query("DELETE FROM agent_room.room_moderation_grant WHERE id = $1")
        .bind(grant.id.as_uuid())
        .execute(&database.runtime)
        .await
        .expect("删授权");
    assert!(
        repositories
            .agent_actions(catalog_id)
            .await
            .expect("可读")
            .is_empty()
    );
    assert!(
        ModerationRepository::find_action(&repositories, action.id())
            .await
            .expect("可读")
            .is_some(),
        "台账里的动作照旧留着"
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn agent_解封撤回的封禁和台账一起记下_按请求找得回来_删了授权理由跟着删() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "agent-unban-owner").await;
    let target = seed_principal(&database.runtime, "agent-unban-target").await;
    let agent_id = agent_of(
        &database.runtime,
        &seed_agent(&database.runtime, owner).await,
    )
    .await;
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    PrivateRoomStore::create(
        &repositories,
        &private_room_snapshot(catalog_id, owner),
        time(0),
    )
    .await
    .expect("私人房间夹具应创建");
    let grant = grant(catalog_id, agent_id, owner);
    repositories
        .replace_room_moderation_grant(&grant)
        .await
        .expect("给上");
    let person_ban = applied_ban(&repositories, owner, catalog_id, target, None).await;
    let attribution = |action_id| AgentModerationAttribution {
        action_id,
        agent_id,
        grant_id: grant.id,
        note: AgentModerationNote::parse("连发了五条广告链接。").expect("理由有效"),
    };
    let agent_ban_id = ModerationActionId::from_uuid(Uuid::now_v7());
    let agent_ban = applied_ban(
        &repositories,
        owner,
        catalog_id,
        target,
        Some(attribution(agent_ban_id)),
    )
    .await;
    let bans = repositories.applied_bans(catalog_id).await.expect("可读");
    assert_eq!(
        bans.iter()
            .map(|ban| (ban.action.id(), ban.by_agent))
            .collect::<Vec<_>>(),
        [(person_ban.id(), false), (agent_ban.id(), true)]
    );

    let reversal = AgentModerationReversal {
        request_id: ModerationActionId::from_uuid(Uuid::now_v7()),
        attribution: AgentModerationAttribution {
            note: AgentModerationNote::parse("他说以后不发了。").expect("理由有效"),
            ..attribution(agent_ban.id())
        },
    };
    let mismatched = AgentModerationReversal {
        attribution: attribution(person_ban.id()),
        ..reversal.clone()
    };
    unban_and_check(&repositories, agent_ban, &reversal, &mismatched).await;
    assert_eq!(
        repositories
            .applied_bans(catalog_id)
            .await
            .expect("可读")
            .len(),
        1
    );
    let exported = AccountDeletionRepository::export(&repositories, owner, time(20))
        .await
        .expect("导出")
        .expect("账户还在");
    assert_eq!(
        exported.data["agentModerationReversalNotes"][0]["note"],
        "他说以后不发了。"
    );

    sqlx::query("DELETE FROM agent_room.room_moderation_grant WHERE id = $1")
        .bind(grant.id.as_uuid())
        .execute(&database.runtime)
        .await
        .expect("删授权");
    assert!(
        repositories
            .agent_reversals(catalog_id)
            .await
            .expect("可读")
            .is_empty()
    );
    let kept = ModerationRepository::find_action(&repositories, agent_ban_id)
        .await
        .expect("可读")
        .expect("台账里的撤回照旧留着");
    assert_eq!(kept.status(), ModerationActionStatus::Reversed);
    database.close().await;
}

/// 撤回记不成别的动作的；记成以后按请求 ID 找得回来，列出来带着 Agent 的名字。
async fn unban_and_check(
    repositories: &PostgresRepositories,
    mut ban: ModerationAction,
    reversal: &AgentModerationReversal,
    mismatched: &AgentModerationReversal,
) {
    ban.reverse(time(30)).expect("已生效的能撤");
    let audit = action_audit(
        &ban,
        "moderation.action.reversed",
        ModerationAuditOutcome::Allowed,
    );
    let error = repositories
        .finalize_agent_reversal(&ban, &audit, mismatched)
        .await
        .expect_err("记的不是这个动作");
    assert_eq!(error.kind(), RepositoryErrorKind::Constraint);
    let reversed = repositories
        .finalize_agent_reversal(&ban, &audit, reversal)
        .await
        .expect("撤回");
    assert_eq!(reversed.status(), ModerationActionStatus::Reversed);
    assert_eq!(
        repositories
            .agent_reversal(reversal.request_id)
            .await
            .expect("可读"),
        Some(reversal.clone())
    );
    let records = repositories
        .agent_reversals(ban.room_catalog_id())
        .await
        .expect("可读");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].attribution, reversal.attribution);
    assert_eq!(records[0].agent_display_name, "治理测试 Agent");
}

/// 落下一条封禁；给了 `attribution` 就记成是 Agent 做的（动作 ID 用它的）。
async fn applied_ban(
    repositories: &PostgresRepositories,
    actor: PrincipalId,
    catalog_id: RoomCatalogId,
    target: PrincipalId,
    attribution: Option<AgentModerationAttribution>,
) -> ModerationAction {
    let id = attribution.as_ref().map_or_else(
        || ModerationActionId::from_uuid(Uuid::now_v7()),
        |attribution| attribution.action_id,
    );
    let mut action = ModerationAction::reserve(
        id,
        None,
        actor,
        catalog_id,
        ModerationActionKind::Ban,
        person(target),
        ModerationReason::Spam,
        time(0),
        None,
    )
    .expect("封禁有效");
    let requested = action_audit(
        &action,
        "moderation.action.requested",
        ModerationAuditOutcome::Allowed,
    );
    match &attribution {
        Some(attribution) => repositories
            .reserve_agent_action(&action, &requested, attribution)
            .await
            .expect("预留"),
        None => ModerationRepository::reserve_action(repositories, &action, &requested)
            .await
            .expect("预留"),
    };
    action.mark_applied().expect("落下");
    let applied = action_audit(
        &action,
        "moderation.action.applied",
        ModerationAuditOutcome::Allowed,
    );
    ModerationRepository::finalize_action(repositories, &action, &applied)
        .await
        .expect("落下")
}

async fn no_grant(
    repositories: &PostgresRepositories,
    catalog_id: RoomCatalogId,
    agent_id: AgentId,
) -> bool {
    repositories
        .active_agent_grant(catalog_id, agent_id, time(0))
        .await
        .expect("可读")
        .is_none()
}

fn grant(
    catalog_id: RoomCatalogId,
    agent_id: AgentId,
    granted_by: PrincipalId,
) -> RoomModerationGrant {
    RoomModerationGrant {
        id: RoomModerationGrantId::from_uuid(Uuid::now_v7()),
        catalog_id,
        agent_id,
        granted_by,
        rules: RoomModerationRules::parse("发广告的禁言十分钟。").expect("规则有效"),
        powers: RoomModerationPowers::new([RoomModerationPower::Mute]).expect("勾了一样"),
        created_at: time(0),
        expires_at: time(3_600_000),
        revoked_at: None,
    }
}

fn mute_action(
    actor: PrincipalId,
    catalog_id: RoomCatalogId,
    target: PrincipalId,
) -> ModerationAction {
    ModerationAction::reserve(
        ModerationActionId::from_uuid(Uuid::now_v7()),
        None,
        actor,
        catalog_id,
        ModerationActionKind::Mute,
        person(target),
        ModerationReason::Spam,
        time(0),
        Some(time(600_000)),
    )
    .expect("禁言有效")
}

async fn agent_of(pool: &PgPool, matrix_user_id: &str) -> AgentId {
    let id: Uuid = sqlx::query_scalar("SELECT id FROM agent_room.agent WHERE matrix_user_id = $1")
        .bind(matrix_user_id)
        .fetch_one(pool)
        .await
        .expect("Agent 在");
    AgentId::from_uuid(id)
}

/// 替网络 Agent 记账的合成主体。
async fn seed_network_principal(pool: &PgPool) -> PrincipalId {
    let principal_id = seed_principal(pool, "network-agent").await;
    sqlx::query("UPDATE agent_room.principal SET oidc_issuer = $2 WHERE id = $1")
        .bind(principal_id.as_uuid())
        .bind(NETWORK_AGENT_ISSUER)
        .execute(pool)
        .await
        .expect("改成网络 Agent 的主体");
    principal_id
}
