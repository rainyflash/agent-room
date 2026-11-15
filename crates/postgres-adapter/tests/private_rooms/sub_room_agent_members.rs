//! 子房间里的 Agent 成员（2f-2b）：父房间里移出它时子房间里跟着移出、排上踢出去；子房间里只记父房间里
//! 此刻还在的 Agent 成员。测试库是一路用下来的，排着的事只领自己那间房下面的。

use agent_room_application::ports::{
    PrivateRoomAgentAccessStore, SubRoomDoorSource, SubRoomStore, SubRoomSyncScope,
};
use agent_room_domain::join_codes::{PrivateRoomAgentJoinedVia, PrivateRoomAgentMemberStatus};

use super::{sync::open, *};

async fn admit_agent(
    repositories: &PostgresRepositories,
    room: RoomCatalogId,
    agent: AgentId,
    via: PrivateRoomAgentJoinedVia,
    at: i64,
) {
    PrivateRoomAgentAccessStore::admit_agent(
        repositories,
        room,
        agent,
        PrivateRoomPermissions::AGENT_MEMBER,
        via,
        time(at),
    )
    .await
    .expect("记得上");
}

async fn status(
    repositories: &PostgresRepositories,
    room: RoomCatalogId,
    agent: AgentId,
) -> Option<PrivateRoomAgentMemberStatus> {
    PrivateRoomAgentAccessStore::agent_member(repositories, room, agent)
        .await
        .expect("读得出")
        .map(|member| member.status)
}

async fn remove(
    repositories: &PostgresRepositories,
    room: RoomCatalogId,
    agent: AgentId,
    at: i64,
) -> bool {
    PrivateRoomAgentAccessStore::remove_agent(repositories, room, agent, time(at))
        .await
        .expect("记得上")
}

/// 这间子房间里排着的 Agent 的事：哪个 Agent、第几版。
async fn queued_agents(pool: &PgPool, sub_room: RoomCatalogId) -> Vec<(AgentId, i64)> {
    let rows: Vec<(Uuid, i64)> = sqlx::query_as(
        r"SELECT agent_id, generation
            FROM agent_room.private_room_sub_room_sync
           WHERE catalog_entry_id = $1
             AND agent_id IS NOT NULL
           ORDER BY agent_id",
    )
    .bind(sub_room.as_uuid())
    .fetch_all(pool)
    .await
    .expect("排着的事可读");
    rows.into_iter()
        .map(|(agent, generation)| (AgentId::from_uuid(agent), generation))
        .collect()
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 父房间里移出_agent_成员时_子房间里跟着移出_排上踢出去_领到的是这个_agent() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-agent-follow-owner").await;
    let agent_owner = seed_principal(pool, "sub-room-agent-follow-agent-owner").await;
    let agent = seed_agent(pool, agent_owner).await;
    let other = seed_agent(pool, agent_owner).await;
    let parent = create_room(&repositories, owner).await;
    let alpha = open(&repositories, parent, "Alpha", 10).await;
    let beta = open(&repositories, parent, "Beta", 11).await;
    for each in [agent, other] {
        admit_agent(
            &repositories,
            parent,
            each,
            PrivateRoomAgentJoinedVia::Code,
            20,
        )
        .await;
        admit_agent(
            &repositories,
            alpha,
            each,
            PrivateRoomAgentJoinedVia::Knock,
            30,
        )
        .await;
    }
    assert_eq!(
        SubRoomDoorSource::agent_sub_rooms(&repositories, parent, agent)
            .await
            .expect("读得出"),
        [alpha],
        "没进的那一间不算"
    );

    assert!(remove(&repositories, parent, agent, 40).await);
    let removed = Some(PrivateRoomAgentMemberStatus::Removed);
    assert_eq!(status(&repositories, parent, agent).await, removed);
    assert_eq!(status(&repositories, alpha, agent).await, removed);
    assert_eq!(status(&repositories, beta, agent).await, None);
    assert_eq!(
        status(&repositories, alpha, other).await,
        Some(PrivateRoomAgentMemberStatus::Joined),
        "别的 Agent 不动"
    );
    assert_eq!(queued_agents(pool, alpha).await, [(agent, 1)]);
    assert!(queued_agents(pool, beta).await.is_empty());
    assert!(
        SubRoomDoorSource::agent_sub_rooms(&repositories, parent, agent)
            .await
            .expect("读得出")
            .is_empty()
    );

    let under = SubRoomSyncScope::Under(parent);
    let tasks = SubRoomStore::claim_sub_room_sync(&repositories, under, 10, time(100), time(200))
        .await
        .expect("领得到");
    assert_eq!(tasks.len(), 1, "{tasks:?}");
    let task = tasks[0];
    assert_eq!(
        (task.catalog_id, task.principal, task.agent),
        (alpha, None, Some(agent))
    );
    SubRoomStore::complete_sub_room_sync(&repositories, task)
        .await
        .expect("记得上");
    assert!(queued_agents(pool, alpha).await.is_empty());
    // 已经被别处做完删掉了再报做完：照这个 Agent 再排一次，不当成这间房自己的事。
    SubRoomStore::complete_sub_room_sync(&repositories, task)
        .await
        .expect("记得上");
    assert_eq!(queued_agents(pool, alpha).await, [(agent, 1)]);

    // 父房间里已经移出了的再移出一次：子房间里没有已加入的，什么也不排。
    assert!(!remove(&repositories, parent, agent, 50).await);
    assert_eq!(queued_agents(pool, alpha).await, [(agent, 1)]);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 子房间里只记父房间里此刻还在的_agent_成员_父房间照旧随便记() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-agent-gate-owner").await;
    let agent_owner = seed_principal(pool, "sub-room-agent-gate-agent-owner").await;
    let agent = seed_agent(pool, agent_owner).await;
    let parent = create_room(&repositories, owner).await;
    let alpha = open(&repositories, parent, "Alpha", 10).await;

    admit_agent(
        &repositories,
        alpha,
        agent,
        PrivateRoomAgentJoinedVia::Knock,
        20,
    )
    .await;
    assert_eq!(
        status(&repositories, alpha, agent).await,
        None,
        "不是父房间里的 Agent 成员"
    );

    admit_agent(
        &repositories,
        parent,
        agent,
        PrivateRoomAgentJoinedVia::Code,
        30,
    )
    .await;
    assert!(remove(&repositories, parent, agent, 40).await);
    admit_agent(
        &repositories,
        alpha,
        agent,
        PrivateRoomAgentJoinedVia::Knock,
        50,
    )
    .await;
    assert_eq!(
        status(&repositories, alpha, agent).await,
        None,
        "父房间里已经移出了"
    );

    admit_agent(
        &repositories,
        parent,
        agent,
        PrivateRoomAgentJoinedVia::Code,
        60,
    )
    .await;
    admit_agent(
        &repositories,
        alpha,
        agent,
        PrivateRoomAgentJoinedVia::Knock,
        70,
    )
    .await;
    assert_eq!(
        status(&repositories, alpha, agent).await,
        Some(PrivateRoomAgentMemberStatus::Joined)
    );

    database.close().await;
}
