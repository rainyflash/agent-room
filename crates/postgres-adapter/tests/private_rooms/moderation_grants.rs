//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：只认主人名下还在用的
//! Agent；给了读得回来；同一个 Agent 再给一次，上一份在同一个事务里记成被撤了；到期了、撤了的不列。

use agent_room_application::ports::{RoomModerationGrant, RoomModerationGrantStore};
use agent_room_domain::{
    ids::RoomModerationGrantId,
    room_moderation_grants::{RoomModerationPower, RoomModerationPowers, RoomModerationRules},
};

use super::{listing::create_room, *};

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 只认主人名下的_agent_再给一次盖掉上一份_到期和撤了的不列() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "grant-owner").await;
    let other = seed_principal(&database.runtime, "grant-other").await;
    let agent = seed_agent(&database.runtime, owner).await;
    let others_agent = seed_agent(&database.runtime, other).await;
    assert_eq!(
        repositories
            .owned_agent_name(owner, agent)
            .await
            .expect("可读")
            .as_deref(),
        Some("建房测试 Agent")
    );
    assert_eq!(
        repositories
            .owned_agent_name(owner, others_agent)
            .await
            .expect("可读"),
        None
    );
    let room = create_room(&repositories, owner).await;

    let first = grant(room, agent, owner, 100);
    repositories
        .replace_room_moderation_grant(&first)
        .await
        .expect("给上");
    let active = repositories
        .active_room_moderation_grants(room, time(150))
        .await
        .expect("可读");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].grant, first);
    assert_eq!(active[0].agent_display_name, "建房测试 Agent");

    let second = grant(room, agent, owner, 200);
    repositories
        .replace_room_moderation_grant(&second)
        .await
        .expect("再给一次");
    let replaced = repositories
        .room_moderation_grant(first.id)
        .await
        .expect("可读")
        .expect("还留着");
    assert_eq!(replaced.revoked_at, Some(time(200)));
    let active = repositories
        .active_room_moderation_grants(room, time(250))
        .await
        .expect("可读");
    assert_eq!(
        active
            .iter()
            .map(|active| active.grant.id)
            .collect::<Vec<_>>(),
        [second.id]
    );
    assert!(
        repositories
            .active_room_moderation_grants(room, second.expires_at)
            .await
            .expect("可读")
            .is_empty(),
        "到期了不列"
    );

    assert!(
        repositories
            .revoke_room_moderation_grant(second.id, owner, time(300))
            .await
            .expect("撤掉")
    );
    assert!(
        !repositories
            .revoke_room_moderation_grant(second.id, owner, time(400))
            .await
            .expect("撤过的不再改")
    );
    let revoked = repositories
        .room_moderation_grant(second.id)
        .await
        .expect("可读")
        .expect("还留着");
    assert_eq!(revoked.revoked_at, Some(time(300)));
    assert!(
        repositories
            .active_room_moderation_grants(room, time(350))
            .await
            .expect("可读")
            .is_empty()
    );
    database.close().await;
}

fn grant(
    catalog_id: RoomCatalogId,
    agent_id: AgentId,
    granted_by: PrincipalId,
    created: i64,
) -> RoomModerationGrant {
    RoomModerationGrant {
        id: RoomModerationGrantId::from_uuid(Uuid::now_v7()),
        catalog_id,
        agent_id,
        granted_by,
        rules: RoomModerationRules::parse("说 spam 的禁言 10 分钟。\n发广告链接的封禁。")
            .expect("规则有效"),
        powers: RoomModerationPowers::new([RoomModerationPower::Mute, RoomModerationPower::Ban])
            .expect("勾了两样"),
        created_at: time(created),
        expires_at: time(created + 3_600_000),
        revoked_at: None,
    }
}
