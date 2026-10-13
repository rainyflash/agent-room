//! 网络 Agent 自己建房间、管房间（`specs/agent-rooms/design.md` 第 4 步）。

use agent_room_application::network_agents::{
    NetworkAgentFailure, NetworkAgentOwnedRoom, NetworkAgentRoomCode, NetworkAgentRoomCodeView,
    NetworkAgentRoomUseCases,
};

use super::*;

/// 起一个在公开大厅里的网络 Agent，拿到它的令牌。
async fn agent(harness: &Harness) -> String {
    let created = harness.create("Scout", None).await.expect("起名成功");
    created.token.expose().to_owned()
}

async fn create_room(
    harness: &Harness,
    token: &str,
    name: &str,
) -> Result<NetworkAgentOwnedRoom, NetworkAgentFailureKind> {
    harness
        .service
        .create_room(token, name)
        .await
        .map_err(|failure| failure.kind())
}

async fn close_room(
    harness: &Harness,
    token: &str,
    room: &str,
) -> Result<(), NetworkAgentFailureKind> {
    harness
        .service
        .close_room(token, room)
        .await
        .map_err(|failure| failure.kind())
}

fn visible(code: &NetworkAgentRoomCode) -> &str {
    match code {
        NetworkAgentRoomCode::Visible(code) => code,
        NetworkAgentRoomCode::Hidden | NetworkAgentRoomCode::Missing => panic!("口令应当看得到"),
    }
}

async fn room_code(
    harness: &Harness,
    token: &str,
    room: &str,
) -> Result<NetworkAgentRoomCodeView, NetworkAgentFailureKind> {
    harness
        .service
        .room_code(token, room)
        .await
        .map_err(|failure| failure.kind())
}

fn waiting_names(knocks: &[PrivateRoomAgentKnockRecord]) -> Vec<&str> {
    knocks
        .iter()
        .map(|knock| knock.display_name.as_str())
        .collect()
}

async fn record_joined(harness: &Harness, room: &NetworkAgentRoom) {
    let id = harness.store.only().id;
    harness
        .store
        .record_room(
            id,
            &NetworkAgentRoomRecord {
                catalog_id: room.catalog_id,
                matrix_room_id: room.matrix_room_id.clone(),
                joined_at: UtcMillis::new(START).unwrap(),
                name: None,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn 建房_房主是它自己_记下建房的_agent_顺手生成口令_还没进去() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let record = harness.store.only();

    let owned = create_room(&harness, &token, "  周报 ").await.unwrap();

    assert!(owned.created);
    assert!(!owned.joined, "由网关让加密客户端就绪后再进");
    assert_eq!(owned.room.name, "周报", "名字去掉首尾空白");
    assert_eq!(
        owned.room.matrix_room_id,
        owned_matrix_room(owned.room.catalog_id)
    );
    {
        let created = harness.rooms.created.lock().unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(
            created[0].actor.principal_id(),
            record.principal_id,
            "房主是它的合成主体"
        );
        assert_eq!(created[0].created_by_agent, record.agent_id);
        assert_eq!(created[0].catalog_id, owned.room.catalog_id);
        assert!(created[0].invitations.is_empty());
    }
    assert_eq!(
        *harness.access.managed.lock().unwrap(),
        [(record.principal_id, owned.room.catalog_id, "generate")],
        "新房间直接生成口令，不先看"
    );
    assert_eq!(
        visible(&owned.code),
        harness.access.codes.lock().unwrap()[&owned.room.catalog_id].display()
    );
}

#[tokio::test]
async fn 同名的没关的房间回到那间_口令照旧给_进过的标出来() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let first = create_room(&harness, &token, "周报").await.unwrap();

    let again = create_room(&harness, &token, "周报 ").await.unwrap();
    assert!(!again.created);
    assert!(!again.joined, "上次没进去，这次由网关接着进");
    assert_eq!(again.room, first.room);
    assert_eq!(visible(&again.code), visible(&first.code));
    assert_eq!(harness.rooms.created.lock().unwrap().len(), 1, "没多建一间");
    assert_eq!(harness.access.generated_codes(), 1, "看得到就不换口令");

    record_joined(&harness, &first.room).await;
    assert!(
        create_room(&harness, &token, "周报").await.unwrap().joined,
        "已经在里面：网关不用再进一次"
    );

    let other = create_room(&harness, &token, "Weekly").await.unwrap();
    assert!(other.created);
    assert_ne!(other.room.catalog_id, first.room.catalog_id);
}

#[tokio::test]
async fn 口令看不到时不替它换_没有了就再生成一个() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let first = create_room(&harness, &token, "周报").await.unwrap();
    let catalog_id = first.room.catalog_id;

    harness.access.hidden.lock().unwrap().insert(catalog_id);
    let hidden = create_room(&harness, &token, "周报").await.unwrap();
    assert_eq!(hidden.code, NetworkAgentRoomCode::Hidden);
    assert_eq!(
        harness.access.generated_codes(),
        1,
        "换了口令，已经拿到旧口令、还没进来的 Agent 就进不来了"
    );

    harness.access.hidden.lock().unwrap().clear();
    harness.access.codes.lock().unwrap().clear();
    let regenerated = create_room(&harness, &token, "周报").await.unwrap();
    assert_eq!(harness.access.generated_codes(), 2);
    assert_ne!(visible(&regenerated.code), visible(&first.code));
}

#[tokio::test]
async fn 房间名不对时什么都不建_也不占当天的名额() {
    let harness = Harness::new(NetworkAgentPolicy {
        rooms_per_agent_per_day: 1,
        ..NetworkAgentPolicy::default_limits(true)
    });
    let token = agent(&harness).await;

    for name in ["", "   ", &"长".repeat(129), "周\n报"] {
        assert_eq!(
            create_room(&harness, &token, name).await.unwrap_err(),
            NetworkAgentFailureKind::RoomNameInvalid,
            "{name:?}"
        );
    }
    assert!(harness.rooms.created.lock().unwrap().is_empty());
    assert!(
        create_room(&harness, &token, &"长".repeat(128))
            .await
            .unwrap()
            .created,
        "128 个字符可以，当天的名额还在"
    );
}

#[tokio::test]
async fn 每个网络_agent_同时五个没关的房间_关掉一个就能再建() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let mut rooms = Vec::new();
    for index in 0..5 {
        rooms.push(
            create_room(&harness, &token, &format!("房间 {index}"))
                .await
                .unwrap(),
        );
    }

    assert_eq!(
        create_room(&harness, &token, "第六间").await.unwrap_err(),
        NetworkAgentFailureKind::RoomLimitReached
    );
    assert_eq!(
        create_room(&harness, &token, "房间 0").await.unwrap().room,
        rooms[0].room,
        "回到已有的同名房间不算新建"
    );

    close_room(&harness, &token, &rooms[0].room.catalog_id.to_string())
        .await
        .unwrap();
    assert!(
        create_room(&harness, &token, "第六间")
            .await
            .unwrap()
            .created
    );
}

#[tokio::test]
async fn 每天最多建十个_超了告诉什么时候能再建() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    for index in 0..10 {
        let owned = create_room(&harness, &token, &format!("房间 {index}"))
            .await
            .unwrap();
        close_room(&harness, &token, &owned.room.catalog_id.to_string())
            .await
            .unwrap();
    }

    let failure = harness
        .service
        .create_room(&token, "第十一间")
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), NetworkAgentFailureKind::RateLimited);
    assert_eq!(
        failure.retry_at(),
        Some(UtcMillis::new(START + 24 * HOUR).unwrap())
    );

    harness.runtime.advance(24 * HOUR);
    assert!(
        create_room(&harness, &token, "第十一间")
            .await
            .unwrap()
            .created
    );
}

#[tokio::test]
async fn 全站网络_agent_建的没关的房间满了就请稍后_不占当天的名额() {
    let harness = Harness::new(NetworkAgentPolicy {
        rooms_per_agent_per_day: 1,
        ..NetworkAgentPolicy::default_limits(true)
    });
    let token = agent(&harness).await;
    *harness.store.owned_private_rooms.lock().unwrap() = 500;

    assert_eq!(
        create_room(&harness, &token, "周报").await.unwrap_err(),
        NetworkAgentFailureKind::RoomCapacityReached
    );
    assert!(harness.rooms.created.lock().unwrap().is_empty());

    *harness.store.owned_private_rooms.lock().unwrap() = 499;
    assert!(create_room(&harness, &token, "周报").await.unwrap().created);
}

/// 凭口令新起一个网络 Agent，从 `source` 这个来源。
async fn create_with_code_from(
    harness: &Harness,
    name: &str,
    source_digest: [u8; 32],
) -> Result<CreatedNetworkAgent, NetworkAgentFailure> {
    harness
        .service
        .create(CreateNetworkAgent {
            name: name.to_owned(),
            room: NetworkAgentRoomRequest::Code(CODE.to_owned()),
            source_digest,
        })
        .await
}

#[tokio::test]
async fn 凭网络_agent_建的房间的口令新起_agent_和进大厅共用起名的名额_人管的房间不算() {
    let harness = Harness::new(NetworkAgentPolicy {
        lobby_entries_per_source_per_hour: 2,
        ..NetworkAgentPolicy::default_limits(true)
    });
    for index in 0..3 {
        create_with_code_from(&harness, &format!("Guest {index}"), [1; 32])
            .await
            .unwrap_or_else(|failure| {
                panic!("人管的房间的口令不占名额，第 {index} 个：{failure:?}")
            });
    }

    harness
        .store
        .network_owned
        .lock()
        .unwrap()
        .insert(private_redeemed().catalog_id);
    let greeter = harness.create("Greeter", None).await.expect("进大厅占一个");
    create_with_code_from(&harness, "Cipher", [1; 32])
        .await
        .expect("网络 Agent 建的房间的口令占第二个");
    let agents = harness.store.agents.lock().unwrap().len();
    let limited = create_with_code_from(&harness, "Echo", [1; 32])
        .await
        .unwrap_err();
    assert_eq!(limited.kind(), NetworkAgentFailureKind::RateLimited);
    assert_eq!(
        limited.retry_at(),
        Some(UtcMillis::new(START + HOUR).unwrap())
    );
    assert_eq!(
        harness.store.agents.lock().unwrap().len(),
        agents,
        "名额用完了什么都不建"
    );

    // 只算新起的：已经有的网络 Agent 凭这个口令进来不占名额。名额按来源算，别的来源照常。
    let admitted = harness
        .service
        .admit(
            greeter.token.expose(),
            NetworkAgentRoomRequest::Code(CODE.to_owned()),
            [1; 32],
        )
        .await
        .expect("已有的网络 Agent 凭口令进来");
    assert!(matches!(
        admitted,
        NetworkAgentAdmission::Admitted(NetworkAgentTarget::Private(_))
    ));
    create_with_code_from(&harness, "Delta", [2; 32])
        .await
        .expect("别的来源照常起");

    harness.runtime.advance(HOUR);
    create_with_code_from(&harness, "Echo", [1; 32])
        .await
        .expect("过了一小时又能起");
}

#[tokio::test]
async fn 关房间只认它当房主的_房间号和_room_id_都行_关了的再关一次也成功() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let record = harness.store.only();
    let owned = create_room(&harness, &token, "周报").await.unwrap();
    let number = owned.room.catalog_id.to_string();

    close_room(&harness, &token, &number).await.unwrap();
    close_room(
        &harness,
        &token,
        &format!("https://agentroom.chat/lobby/{number}"),
    )
    .await
    .unwrap();
    assert_eq!(
        *harness.rooms.archived.lock().unwrap(),
        [(record.principal_id, owned.room.catalog_id); 2]
    );
    assert!(
        create_room(&harness, &token, "周报").await.unwrap().created,
        "关了的不算同名"
    );

    let second = create_room(&harness, &token, "第二间").await.unwrap();
    record_joined(&harness, &second.room).await;
    close_room(&harness, &token, second.room.matrix_room_id.as_str())
        .await
        .unwrap();
    assert_eq!(harness.rooms.archived.lock().unwrap().len(), 3);

    let others = harness
        .rooms
        .seed_other(PrincipalId::from_uuid(Uuid::now_v7()));
    for room in [
        others.to_string(),
        catalog_id(78).to_string(),
        "!unknown:matrix.test".to_owned(),
        "不是房间".to_owned(),
    ] {
        assert_eq!(
            close_room(&harness, &token, &room).await.unwrap_err(),
            NetworkAgentFailureKind::RoomNotOwned,
            "别人的房间、没有的房间、没进过的房间都只说不是它的：{room}"
        );
    }
    assert_eq!(harness.rooms.archived.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn 看口令不替它生成_停用了说没有_看不到的说看不到() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let owned = create_room(&harness, &token, "周报").await.unwrap();
    let number = owned.room.catalog_id.to_string();

    let shown = room_code(&harness, &token, &number).await.unwrap();
    assert_eq!(shown.room, owned.room);
    assert_eq!(visible(&shown.code), visible(&owned.code));

    harness.service.disable_code(&token, &number).await.unwrap();
    assert_eq!(
        room_code(&harness, &token, &number).await.unwrap().code,
        NetworkAgentRoomCode::Missing
    );
    harness
        .service
        .disable_code(&token, &number)
        .await
        .expect("没有口令时再停用也成功");
    assert_eq!(
        harness.access.generated_codes(),
        1,
        "看口令没有副作用：没有就说没有，由它决定要不要生成"
    );

    let replaced = harness.service.replace_code(&token, &number).await.unwrap();
    assert_eq!(replaced.room, owned.room);
    assert_ne!(visible(&replaced.code), visible(&owned.code));
    assert_eq!(
        room_code(&harness, &token, &number).await.unwrap().code,
        replaced.code
    );

    harness
        .access
        .hidden
        .lock()
        .unwrap()
        .insert(owned.room.catalog_id);
    assert_eq!(
        room_code(&harness, &token, &number).await.unwrap().code,
        NetworkAgentRoomCode::Hidden
    );
}

#[tokio::test]
async fn 管房间只认它当房主_还没关的房间_也认它的_matrix_房间_id() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let owned = create_room(&harness, &token, "周报").await.unwrap();
    assert_eq!(
        room_code(&harness, &token, owned.room.matrix_room_id.as_str())
            .await
            .unwrap()
            .room,
        owned.room,
        "建好还没进去时也认得出 Matrix 房间 ID"
    );

    let closed = create_room(&harness, &token, "关了的").await.unwrap();
    close_room(&harness, &token, &closed.room.catalog_id.to_string())
        .await
        .unwrap();
    let others = harness
        .rooms
        .seed_other(PrincipalId::from_uuid(Uuid::now_v7()));
    let knocker = AgentId::from_uuid(Uuid::now_v7());
    let before = harness.access.managed.lock().unwrap().len();
    for room in [
        closed.room.catalog_id.to_string(),
        others.to_string(),
        catalog_id(78).to_string(),
        "!unknown:matrix.test".to_owned(),
    ] {
        assert_eq!(
            room_code(&harness, &token, &room).await.unwrap_err(),
            NetworkAgentFailureKind::RoomNotOwned,
            "{room}"
        );
        for failure in [
            harness
                .service
                .replace_code(&token, &room)
                .await
                .unwrap_err(),
            harness
                .service
                .disable_code(&token, &room)
                .await
                .unwrap_err(),
            harness
                .service
                .waiting_knocks(&token, &room)
                .await
                .unwrap_err(),
            harness
                .service
                .admit_knock(&token, &room, knocker)
                .await
                .unwrap_err(),
            harness
                .service
                .decline_knock(&token, &room, knocker)
                .await
                .unwrap_err(),
        ] {
            assert_eq!(
                failure.kind(),
                NetworkAgentFailureKind::RoomNotOwned,
                "{room}"
            );
        }
    }
    assert_eq!(
        harness.access.managed.lock().unwrap().len(),
        before,
        "别人的房间、关了的房间碰都不碰"
    );
}

#[tokio::test]
async fn 看谁在敲门_放行以后等网关进去了再记放进来了_不让进的再放行说敲门不在了() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let owned = create_room(&harness, &token, "周报").await.unwrap();
    let catalog_id = owned.room.catalog_id;
    let number = catalog_id.to_string();
    let (scout, critic) = (
        AgentId::from_uuid(Uuid::now_v7()),
        AgentId::from_uuid(Uuid::now_v7()),
    );
    let others = harness
        .rooms
        .seed_other(PrincipalId::from_uuid(Uuid::now_v7()));
    harness.access.knocks.lock().unwrap().extend([
        waiting_knock_on(catalog_id, scout, "Scout"),
        waiting_knock_on(catalog_id, critic, "Critic"),
        waiting_knock_on(others, AgentId::from_uuid(Uuid::now_v7()), "别人的"),
    ]);

    let waiting = harness
        .service
        .waiting_knocks(&token, &number)
        .await
        .unwrap();
    assert_eq!(
        waiting_names(&waiting),
        ["Scout", "Critic"],
        "只看这个房间的"
    );

    let admitted = harness
        .service
        .admit_knock(&token, &number, scout)
        .await
        .unwrap();
    assert_eq!(admitted.agent.agent_id, scout);
    assert_eq!(admitted.room.catalog_id, catalog_id);
    assert_eq!(
        waiting_names(
            &harness
                .service
                .waiting_knocks(&token, &number)
                .await
                .unwrap()
        ),
        ["Scout", "Critic"],
        "网关替它进去之前，敲门还在等"
    );
    harness
        .service
        .complete_knock(&token, &number, scout)
        .await
        .unwrap();

    harness
        .service
        .decline_knock(&token, &number, critic)
        .await
        .unwrap();
    assert!(
        harness
            .service
            .waiting_knocks(&token, &number)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        harness
            .service
            .admit_knock(&token, &number, critic)
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::KnockNotFound
    );
    harness
        .service
        .decline_knock(&token, &number, critic)
        .await
        .expect("原本就不在等，再拒绝一次也成功");
}

#[tokio::test]
async fn 邀请人给查看发言和自动发言_没有这个账户说找不到_每天发的有上限() {
    let harness = Harness::new(NetworkAgentPolicy {
        invitations_per_agent_per_day: 2,
        ..NetworkAgentPolicy::default_limits(true)
    });
    let token = agent(&harness).await;
    let record = harness.store.only();
    let owned = create_room(&harness, &token, "周报").await.unwrap();
    let number = owned.room.catalog_id.to_string();
    let person = PrincipalId::from_uuid(Uuid::now_v7());

    harness
        .service
        .invite_person(&token, &number, person)
        .await
        .unwrap();
    {
        let invited = harness.rooms.invited.lock().unwrap();
        assert_eq!(invited.len(), 1);
        assert_eq!(invited[0].actor.principal_id(), record.principal_id);
        assert_eq!(invited[0].catalog_id, owned.room.catalog_id);
        assert_eq!(invited[0].target_principal_id, person);
        assert_eq!(
            invited[0].permissions,
            PrivateRoomPermissions::INVITED_BY_AGENT,
            "给查看、发言和自动发言"
        );
    }

    assert_eq!(
        harness
            .service
            .invite_person(&token, &number, no_account())
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::AccountNotFound
    );
    let limited = harness
        .service
        .invite_person(&token, &number, person)
        .await
        .unwrap_err();
    assert_eq!(
        limited.kind(),
        NetworkAgentFailureKind::RateLimited,
        "找不到的也算一次，免得拿它一个个试账户 ID"
    );
    assert_eq!(
        limited.retry_at(),
        Some(UtcMillis::new(START + 24 * HOUR).unwrap())
    );
    assert_eq!(harness.rooms.invited.lock().unwrap().len(), 2);

    assert_eq!(
        harness
            .service
            .invite_person(&token, &catalog_id(78).to_string(), person)
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::RoomNotOwned,
        "不是它的房间时先说这个，不占名额也不去邀请"
    );
    assert_eq!(harness.rooms.invited.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn 移出凭口令或敲门进来的_agent_再移一次也成功_别的说不在房间里() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let owned = create_room(&harness, &token, "周报").await.unwrap();
    let number = owned.room.catalog_id.to_string();
    let scout = AgentId::from_uuid(Uuid::now_v7());
    harness.access.members.lock().unwrap().insert(scout);

    harness
        .service
        .remove_agent(&token, &number, scout)
        .await
        .unwrap();
    harness
        .service
        .remove_agent(&token, &number, scout)
        .await
        .expect("已经移出的再移一次也成功");
    assert_eq!(
        harness
            .service
            .remove_agent(&token, &number, AgentId::from_uuid(Uuid::now_v7()))
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::AgentNotInRoom
    );
    assert_eq!(
        harness
            .service
            .remove_agent(&token, &catalog_id(78).to_string(), scout)
            .await
            .unwrap_err()
            .kind(),
        NetworkAgentFailureKind::RoomNotOwned
    );
}

#[tokio::test]
async fn 敲它建的房间的门_只列它没关的房间里在等的_先敲的在前_查看自己时也列出() {
    let harness = Harness::enabled();
    let token = agent(&harness).await;
    let nothing = harness.service.incoming_knocks(&token).await.unwrap();
    assert!(!nothing.owns_rooms, "一间房都没有，谁也敲不了它的门");
    assert!(nothing.knocks.is_empty());

    let weekly = create_room(&harness, &token, "周报").await.unwrap().room;
    let review = create_room(&harness, &token, "评审").await.unwrap().room;
    let closed = create_room(&harness, &token, "旧房间").await.unwrap().room;
    close_room(&harness, &token, &closed.catalog_id.to_string())
        .await
        .unwrap();
    let others = harness
        .rooms
        .seed_other(PrincipalId::from_uuid(Uuid::now_v7()));
    let knocker = || AgentId::from_uuid(Uuid::now_v7());
    let at_minute = |knock: PrivateRoomAgentKnockRecord, minute: i64| PrivateRoomAgentKnockRecord {
        knocked_at: UtcMillis::new(START + minute * 60_000).unwrap(),
        ..knock
    };
    harness.access.knocks.lock().unwrap().extend([
        at_minute(waiting_knock_on(review.catalog_id, knocker(), "Critic"), 2),
        at_minute(waiting_knock_on(weekly.catalog_id, knocker(), "Scout"), 1),
        PrivateRoomAgentKnockRecord {
            status: PrivateRoomAgentKnockStatus::Declined,
            ..waiting_knock_on(weekly.catalog_id, knocker(), "没让进的")
        },
        waiting_knock_on(closed.catalog_id, knocker(), "关了的房间"),
        waiting_knock_on(others, knocker(), "别人的房间"),
    ]);

    let incoming = harness.service.incoming_knocks(&token).await.unwrap();
    assert!(incoming.owns_rooms);
    let seen: Vec<(&str, &str)> = incoming
        .knocks
        .iter()
        .map(|incoming| {
            (
                incoming.room.name.as_str(),
                incoming.knock.display_name.as_str(),
            )
        })
        .collect();
    assert_eq!(seen, [("周报", "Scout"), ("评审", "Critic")]);
    assert_eq!(incoming.knocks[0].room, weekly);

    let me = harness.service.me(&token).await.unwrap();
    assert_eq!(me.incoming_knocks, incoming.knocks);
}

#[tokio::test]
async fn 令牌不对或总开关关着时不建也不关() {
    let harness = Harness::enabled();
    agent(&harness).await;
    assert_eq!(
        create_room(&harness, "wrong", "周报").await.unwrap_err(),
        NetworkAgentFailureKind::Unauthorized
    );
    assert_eq!(
        close_room(&harness, "wrong", &catalog_id(9).to_string())
            .await
            .unwrap_err(),
        NetworkAgentFailureKind::Unauthorized
    );
    assert_eq!(
        room_code(&harness, "wrong", &catalog_id(9).to_string())
            .await
            .unwrap_err(),
        NetworkAgentFailureKind::Unauthorized
    );
    assert!(harness.rooms.created.lock().unwrap().is_empty());

    let disabled = Harness::new(NetworkAgentPolicy::default_limits(false));
    assert_eq!(
        create_room(&disabled, "token-1", "周报").await.unwrap_err(),
        NetworkAgentFailureKind::Disabled
    );
}
