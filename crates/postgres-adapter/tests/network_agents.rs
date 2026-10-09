//! 网络 Agent 的真实数据库往返：一个事务写入主体、设备与秘密，没停用的不重名，
//! 生效绑定 Agent 与实例，停用后名字空出来，固定窗口计数；进过的房间与收件箱。

use std::env;

use agent_room_application::{
    persistence::RepositoryErrorKind,
    ports::{
        AgentInstanceManagementRepository, MatrixEventId, MatrixRoomId, MatrixSyncToken,
        MatrixTransactionId, NetworkAgentAckOutcome, NetworkAgentActivation,
        NetworkAgentBeforeJoinGap, NetworkAgentBeginOutcome, NetworkAgentGapReason,
        NetworkAgentHistoryDirection, NetworkAgentHistoryFilter, NetworkAgentHistorySender,
        NetworkAgentInboxAppend, NetworkAgentInboxAppendOutcome, NetworkAgentInboxChange,
        NetworkAgentInboxMessage, NetworkAgentInboxPage, NetworkAgentInboxStore,
        NetworkAgentLookup, NetworkAgentMessageActor, NetworkAgentMessageHistory,
        NetworkAgentMessageRef, NetworkAgentProvisioning, NetworkAgentRecord,
        NetworkAgentRoomRecord, NetworkAgentSecretKind, NetworkAgentStaleCutoff, NetworkAgentStore,
        NetworkAgentStoredMessage, NetworkAgentSubmissionClaim, NetworkAgentSubmissionClaimOutcome,
        NetworkAgentSubmissionKind, NetworkAgentSubmissionState, NetworkAgentSubmissionStore,
        NetworkAgentTimelineGap, PrincipalRegistration, PrivateRoomAgentAccessStore,
        PrivateRoomSnapshot, PrivateRoomStore, RateWindowDecision, RateWindowPolicy, SealedSecret,
        SecretDigest,
    },
};
use agent_room_domain::{
    agents::AgentMatrixDeviceId,
    devices::{Device, DevicePlatform, DevicePublicSigningKey},
    identity::Principal,
    ids::{
        AgentId, AgentInstanceId, DeviceId, MessageId, NetworkAgentId, PrincipalId, RoomCatalogId,
        RoomInstanceId,
    },
    join_codes::{PrivateRoomAgentJoinedVia, PrivateRoomAgentMemberStatus},
    network_agents::{NETWORK_AGENT_ISSUER, NetworkAgentStatus},
    private_rooms::{PrivateRoom, PrivateRoomPermissions},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
    },
    time::{DurationMillis, UtcMillis},
};
use agent_room_postgres_adapter::{PostgresRepositories, run_migrations};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

const HOUR: u64 = 3_600_000;

struct TestDatabase {
    migration: PgPool,
    runtime: PgPool,
}

impl TestDatabase {
    async fn connect() -> Self {
        let migration = connect_pool(&required_url("AGENT_ROOM_TEST_MIGRATION_DATABASE_URL")).await;
        run_migrations(&migration).await.expect("迁移必须成功");
        let runtime = connect_pool(&required_url("AGENT_ROOM_TEST_RUNTIME_DATABASE_URL")).await;
        Self { migration, runtime }
    }

    async fn close(self) {
        self.runtime.close().await;
        self.migration.close().await;
    }
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 一个事务写入主体设备与秘密_没停用的不重名_停用后名字空出() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let name = format!("Scout {}", &Uuid::now_v7().simple().to_string()[..8]);
    let live_before = repositories.count_live().await.expect("计数");

    let first = provisioning(&name, time(0));
    assert_eq!(
        repositories.begin(&first).await.expect("写入"),
        NetworkAgentBeginOutcome::Created
    );
    let record = repositories
        .find_by_token(&first.token_digest)
        .await
        .expect("按令牌找")
        .expect("找得到");
    assert_eq!(record.id, first.id);
    assert_eq!(record.status, NetworkAgentStatus::Provisioning);
    assert_eq!(record.display_name, name);
    assert_eq!(record.agent_id, None);
    assert_eq!(
        repositories.find(first.id).await.expect("按 ID 找"),
        Some(record.clone())
    );
    assert_eq!(
        repositories
            .find(NetworkAgentId::from_uuid(Uuid::now_v7()))
            .await
            .expect("按 ID 找"),
        None
    );
    assert_eq!(
        repositories
            .find_secret(first.id, NetworkAgentSecretKind::InstanceSigningSeed)
            .await
            .expect("读秘密"),
        Some(sealed(2))
    );
    assert_eq!(
        repositories.count_live().await.expect("计数"),
        live_before + 1
    );
    let platform: String =
        sqlx::query_scalar("SELECT platform FROM agent_room.device WHERE id = $1")
            .bind(first.device.id().as_uuid())
            .fetch_one(&database.runtime)
            .await
            .expect("设备已写入");
    assert_eq!(platform, "network");

    // 不分大小写撞名：整个事务回滚，主体也不留下。
    let second = provisioning(&name.to_uppercase(), time(10));
    assert_eq!(
        repositories.begin(&second).await.expect("撞名不算错误"),
        NetworkAgentBeginOutcome::NameTaken
    );
    let leftover: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agent_room.principal WHERE id = $1")
            .bind(second.principal.principal.id().as_uuid())
            .fetch_one(&database.runtime)
            .await
            .expect("查主体");
    assert_eq!(leftover, 0);

    // 停用后令牌不再对应生效的 Agent，名字也空出来。
    repositories
        .disable(first.id, time(20))
        .await
        .expect("停用");
    repositories
        .disable(first.id, time(30))
        .await
        .expect("再停用也行");
    assert_eq!(
        repositories
            .find_by_token(&first.token_digest)
            .await
            .expect("按令牌找")
            .map(|record| record.status),
        Some(NetworkAgentStatus::Disabled)
    );
    let third = provisioning(&name.to_uppercase(), time(40));
    assert_eq!(
        repositories.begin(&third).await.expect("写入"),
        NetworkAgentBeginOutcome::Created
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 生效绑定_agent_与实例_重试同一组也算成功_换一组不行() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let name = format!("Pilot {}", &Uuid::now_v7().simple().to_string()[..8]);
    let provisioning = provisioning(&name, time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let (agent, instance) = seed_agent_instance(
        &database.runtime,
        provisioning.principal.principal.id(),
        provisioning.device.id(),
    )
    .await;
    let activation = NetworkAgentActivation {
        id: provisioning.id,
        agent_id: agent,
        agent_instance_id: instance,
        matrix_access_token: sealed(3),
        activated_at: time(100),
    };
    repositories.activate(&activation).await.expect("生效");
    repositories
        .activate(&activation)
        .await
        .expect("重试同一组");
    let stranger = AgentId::from_uuid(Uuid::now_v7());
    assert_eq!(
        repositories
            .network_agent_ids(&[stranger, agent])
            .await
            .expect("查哪些是网络 Agent"),
        vec![agent]
    );
    assert!(
        repositories
            .network_agent_ids(&[])
            .await
            .expect("空列表不查库")
            .is_empty()
    );
    let record = repositories
        .find_by_token(&provisioning.token_digest)
        .await
        .expect("按令牌找")
        .expect("找得到");
    assert_eq!(record.status, NetworkAgentStatus::Active);
    assert_eq!(record.agent_id, Some(agent));
    assert_eq!(record.agent_instance_id, Some(instance));
    let original = format!("AR_{}", instance.as_uuid().simple());
    assert_eq!(record.matrix_device_id.as_deref(), Some(original.as_str()));
    assert_eq!(record.last_active_at, time(100));

    replaces_matrix_device(&repositories, provisioning.id, instance, &original).await;
    assert_eq!(
        repositories
            .find_secret(provisioning.id, NetworkAgentSecretKind::MatrixAccessToken)
            .await
            .expect("读秘密"),
        Some(sealed(3))
    );
    let (_, other_instance) = seed_agent_instance(
        &database.runtime,
        provisioning.principal.principal.id(),
        provisioning.device.id(),
    )
    .await;
    let conflict = repositories
        .activate(&NetworkAgentActivation {
            agent_instance_id: other_instance,
            ..activation.clone()
        })
        .await
        .expect_err("生效后不能换实例");
    assert_eq!(conflict.kind(), RepositoryErrorKind::Conflict);
    repositories
        .record_activity(provisioning.id, time(50))
        .await
        .expect("更早的活动时间不回退");
    repositories
        .record_activity(provisioning.id, time(500))
        .await
        .expect("记录活动");
    assert_eq!(
        repositories
            .find_by_token(&provisioning.token_digest)
            .await
            .expect("按令牌找")
            .expect("找得到")
            .last_active_at,
        time(500)
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 固定窗口到上限拒绝_窗口过后重新计数() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let bucket = format!("create:hour:{}", Uuid::now_v7().simple());
    let policy = RateWindowPolicy {
        window: DurationMillis::new(HOUR).expect("一小时"),
        limit: 2,
    };
    for offset in [0, 60_000] {
        assert_eq!(
            repositories
                .take(&bucket, time(offset), policy)
                .await
                .expect("计数"),
            RateWindowDecision::Allowed
        );
    }
    let limited = repositories
        .take(&bucket, time(120_000), policy)
        .await
        .expect("计数");
    let hour = i64::try_from(HOUR).expect("一小时");
    assert_eq!(
        limited,
        RateWindowDecision::Limited {
            retry_at: time(hour)
        }
    );
    // 被拒的那次不计数；窗口过去后从这一次重新数。
    assert_eq!(
        repositories
            .take(&bucket, time(hour), policy)
            .await
            .expect("计数"),
        RateWindowDecision::Allowed
    );
    assert_eq!(
        repositories
            .take(&bucket, time(hour + 1), policy)
            .await
            .expect("计数"),
        RateWindowDecision::Allowed
    );
    assert!(matches!(
        repositories
            .take(&bucket, time(hour + 2), policy)
            .await
            .expect("计数"),
        RateWindowDecision::Limited { .. }
    ));
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 进过的房间只记一次_按进入先后列出() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Rover"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let catalog = default_lobby(&database.runtime).await;
    let first = room_record(catalog, "!first:matrix.test", time(10));
    let second = room_record(catalog, "!second:matrix.test", time(20));

    repositories
        .record_room(provisioning.id, &first)
        .await
        .expect("记房间");
    repositories
        .record_room(provisioning.id, &second)
        .await
        .expect("记第二个房间");
    repositories
        .record_room(
            provisioning.id,
            &room_record(catalog, "!first:matrix.test", time(30)),
        )
        .await
        .expect("重复记不报错");

    // 读的时候从房间目录带出房间名；记的时候不用。
    let name: String =
        sqlx::query_scalar("SELECT name FROM agent_room.room_catalog_entry WHERE id = $1")
            .bind(catalog.as_uuid())
            .fetch_one(&database.runtime)
            .await
            .expect("默认公开大厅有名字");
    let named = |record: NetworkAgentRoomRecord| NetworkAgentRoomRecord {
        name: Some(name.clone()),
        ..record
    };
    assert_eq!(
        repositories.rooms(provisioning.id).await.expect("列房间"),
        [named(first), named(second)]
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 收件箱按到达编号_同一事件只收一次_只有作者能改_位置对不上不写() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Scribe"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let id = provisioning.id;
    let edited = MessageId::from_uuid(Uuid::now_v7());
    let withdrawn = MessageId::from_uuid(Uuid::now_v7());
    let kept = MessageId::from_uuid(Uuid::now_v7());

    let empty = repositories.pending(id, None, 10).await.expect("读收件箱");
    assert_eq!(empty.sync_token, None);
    assert!(empty.entries.is_empty());

    let first = append(
        id,
        None,
        "s1",
        vec![
            message("$a:matrix.test", edited, "ranger", "原话"),
            message("$b:matrix.test", withdrawn, "ranger", "撤回的话"),
            message("$c:matrix.test", kept, "ranger", "留着"),
            // 同一事件重复同步到只收一次。
            message("$a:matrix.test", edited, "ranger", "原话"),
            NetworkAgentInboxChange::Replace {
                room_id: room(),
                message_id: edited,
                actor_key: "ranger".to_owned(),
                patch: json!({"title": "改过的话", "conversation": {"text": "改过的话", "mentions": []}}),
            },
            NetworkAgentInboxChange::Redact {
                room_id: room(),
                message_id: withdrawn,
                actor_key: "ranger".to_owned(),
            },
            // 不是作者，改不了也撤不了。
            NetworkAgentInboxChange::Redact {
                room_id: room(),
                message_id: kept,
                actor_key: "impostor".to_owned(),
            },
        ],
        10,
    );
    assert_eq!(
        repositories.append(&first).await.expect("写入同步结果"),
        NetworkAgentInboxAppendOutcome::Applied { appended: 3 }
    );
    // 位置对不上的旧同步不写。
    assert_eq!(
        repositories
            .append(&append(
                id,
                None,
                "s1-late",
                vec![message(
                    "$z:matrix.test",
                    MessageId::from_uuid(Uuid::now_v7()),
                    "ranger",
                    "迟到"
                )],
                10
            ))
            .await
            .expect("写入"),
        NetworkAgentInboxAppendOutcome::Stale
    );

    let page = repositories.pending(id, None, 10).await.expect("读收件箱");
    assert_eq!(
        page.sync_token.as_ref().map(MatrixSyncToken::as_str),
        Some("s1")
    );
    assert_eq!(page.pending, 2);
    assert_eq!(
        texts(
            &page
                .entries
                .iter()
                .map(|entry| entry.preview.clone())
                .collect::<Vec<_>>()
        ),
        ["改过的话", "留着"]
    );
    assert_eq!(page.entries[0].preview["title"], "改过的话");
    assert_eq!(page.entries[0].preview["eventId"], "$a:matrix.test");
    assert!(page.entries[0].sequence < page.entries[1].sequence);
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 收件箱满了丢最早的并计数_确认删到哪条_确认后计数清零() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Keeper"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let id = provisioning.id;
    let early = vec![
        message(
            "$e0:matrix.test",
            MessageId::from_uuid(Uuid::now_v7()),
            "ranger",
            "早一",
        ),
        message(
            "$e1:matrix.test",
            MessageId::from_uuid(Uuid::now_v7()),
            "ranger",
            "早二",
        ),
    ];
    repositories
        .append(&append(id, None, "s1", early, 3))
        .await
        .expect("写入");

    let overflow = (0..3)
        .map(|index| {
            message(
                &format!("$o{index}:matrix.test"),
                MessageId::from_uuid(Uuid::now_v7()),
                "ranger",
                &format!("第 {index} 条"),
            )
        })
        .collect();
    assert_eq!(
        repositories
            .append(&append(id, Some("s1"), "s2", overflow, 3))
            .await
            .expect("写入"),
        NetworkAgentInboxAppendOutcome::Applied { appended: 3 }
    );
    let page = repositories.pending(id, None, 10).await.expect("读收件箱");
    assert_eq!(page.pending, 3);
    assert_eq!(page.dropped, 2);
    assert_eq!(
        texts(
            &page
                .entries
                .iter()
                .map(|entry| entry.preview.clone())
                .collect::<Vec<_>>()
        ),
        ["第 0 条", "第 1 条", "第 2 条"]
    );

    assert_eq!(
        repositories
            .acknowledge(id, &MatrixEventId::new("$o1:matrix.test").unwrap(), None)
            .await
            .expect("确认"),
        NetworkAgentAckOutcome::Acknowledged { pending: 1 }
    );
    assert_eq!(
        repositories
            .acknowledge(id, &MatrixEventId::new("$o0:matrix.test").unwrap(), None)
            .await
            .expect("确认过的再确认"),
        NetworkAgentAckOutcome::NotPending { pending: 1 }
    );
    let page = repositories.pending(id, None, 10).await.expect("读收件箱");
    assert_eq!(page.dropped, 0);
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].event_id.as_str(), "$o2:matrix.test");
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 可以只读只确认一个房间_不给房间就是所有房间里在它之前到的() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Roamer"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let id = provisioning.id;
    let (lobby, den) = (lobby(), den());
    repositories
        .append(&append(
            id,
            None,
            "s1",
            vec![
                said("$l1:matrix.test", &lobby),
                said("$d1:matrix.test", &den),
                said("$l2:matrix.test", &lobby),
                said("$d2:matrix.test", &den),
                said("$l3:matrix.test", &lobby),
            ],
            10,
        ))
        .await
        .expect("写入");

    let page = repositories
        .pending(id, Some(&lobby), 2)
        .await
        .expect("只读大厅");
    assert_eq!(page.pending, 3, "pending 只算这个房间的");
    assert_eq!(event_ids(&page), ["$l1:matrix.test", "$l2:matrix.test"]);

    // 只确认大厅的：另一个房间里更早到的 $d1 还留着。别的房间里的那条不算这个房间的。
    let event = |id: &str| MatrixEventId::new(id).expect("事件 ID 有效");
    assert_eq!(
        repositories
            .acknowledge(id, &event("$l2:matrix.test"), Some(&lobby))
            .await
            .expect("确认大厅"),
        NetworkAgentAckOutcome::Acknowledged { pending: 1 }
    );
    assert_eq!(
        repositories
            .acknowledge(id, &event("$d1:matrix.test"), Some(&lobby))
            .await
            .expect("确认"),
        NetworkAgentAckOutcome::NotPending { pending: 1 }
    );
    assert_eq!(
        event_ids(&repositories.pending(id, None, 10).await.expect("读收件箱")),
        ["$d1:matrix.test", "$d2:matrix.test", "$l3:matrix.test"]
    );

    assert_eq!(
        repositories
            .acknowledge(id, &event("$d2:matrix.test"), None)
            .await
            .expect("确认"),
        NetworkAgentAckOutcome::Acknowledged { pending: 1 }
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 每个房间各自限额_满了只丢这个房间最早的() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Hoarder"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let id = provisioning.id;
    let (lobby, den) = (lobby(), den());
    repositories
        .append(&append(
            id,
            None,
            "s1",
            vec![
                said("$d1:matrix.test", &den),
                said("$d2:matrix.test", &den),
                said("$l1:matrix.test", &lobby),
                said("$l2:matrix.test", &lobby),
                said("$l3:matrix.test", &lobby),
            ],
            2,
        ))
        .await
        .expect("写入");

    let page = repositories.pending(id, None, 10).await.expect("读收件箱");
    assert_eq!(page.dropped, 1);
    assert_eq!(
        event_ids(&page),
        [
            "$d1:matrix.test",
            "$d2:matrix.test",
            "$l2:matrix.test",
            "$l3:matrix.test"
        ]
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 停用以后收件箱不再写入_记下离开房间时删掉留下的消息() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Drifter"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let id = provisioning.id;
    let (agent, instance) = seed_agent_instance(
        &database.runtime,
        provisioning.principal.principal.id(),
        provisioning.device.id(),
    )
    .await;
    repositories
        .activate(&NetworkAgentActivation {
            id,
            agent_id: agent,
            agent_instance_id: instance,
            matrix_access_token: sealed(5),
            activated_at: time(0),
        })
        .await
        .expect("生效");
    repositories
        .append(&append(
            id,
            None,
            "s1",
            vec![said("$kept:matrix.test", &lobby())],
            10,
        ))
        .await
        .expect("写入");

    repositories.disable(id, time(10)).await.expect("停用");
    // 停用前就开始的长轮询这时才同步完：不再写。
    assert_eq!(
        repositories
            .append(&append(
                id,
                Some("s1"),
                "s2",
                vec![said("$late:matrix.test", &lobby())],
                10
            ))
            .await
            .expect("写入"),
        NetworkAgentInboxAppendOutcome::Stale
    );
    assert_eq!(
        repositories
            .pending(id, None, 10)
            .await
            .expect("读收件箱")
            .pending,
        1,
        "还没离开房间时先留着"
    );

    repositories
        .mark_rooms_left(id, time(20))
        .await
        .expect("记下已离开");
    assert_eq!(
        repositories
            .pending(id, None, 10)
            .await
            .expect("读收件箱")
            .pending,
        0
    );
    assert!(
        repositories
            .messages_by_id(id, &[event_ref("$kept:matrix.test")])
            .await
            .expect("读消息记录")
            .is_empty(),
        "消息记录也删掉"
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 消息记录留着确认过的和自己发的_按_id_取_重复同步到的不再进收件箱() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Recorder"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let id = provisioning.id;
    let lobby = lobby();
    let theirs = MessageId::from_uuid(Uuid::now_v7());
    let changes = vec![
        authored(
            "$mine:matrix.test",
            &lobby,
            MessageId::from_uuid(Uuid::now_v7()),
            "scout",
            true,
            false,
        ),
        authored(
            "$theirs:matrix.test",
            &lobby,
            theirs,
            "ranger",
            false,
            false,
        ),
        said("$den:matrix.test", &den()),
    ];
    assert_eq!(
        repositories
            .append(&append(id, None, "s1", changes.clone(), 10))
            .await
            .expect("写入"),
        NetworkAgentInboxAppendOutcome::Applied { appended: 2 },
        "自己发的不进收件箱"
    );
    repositories
        .acknowledge(
            id,
            &MatrixEventId::new("$den:matrix.test").expect("事件 ID 有效"),
            None,
        )
        .await
        .expect("确认");
    assert_eq!(
        repositories
            .pending(id, None, 10)
            .await
            .expect("读")
            .pending,
        0
    );

    let found = repositories
        .messages_by_id(
            id,
            &[
                event_ref("$mine:matrix.test"),
                NetworkAgentMessageRef::Message(theirs),
                event_ref("$den:matrix.test"),
                event_ref("$nope:matrix.test"),
            ],
        )
        .await
        .expect("按 ID 取");
    assert_eq!(
        stored_ids(&found),
        [
            "$mine:matrix.test",
            "$theirs:matrix.test",
            "$den:matrix.test"
        ],
        "按到达先后"
    );
    assert_eq!(found[2].room_id, den());

    // 同一批又同步到一次（比如换了位置重来）：记过的不再记，确认过的也不再进收件箱。
    assert_eq!(
        repositories
            .append(&append(id, Some("s1"), "s2", changes, 10))
            .await
            .expect("写入"),
        NetworkAgentInboxAppendOutcome::Applied { appended: 0 }
    );
    assert_eq!(
        repositories
            .pending(id, None, 10)
            .await
            .expect("读")
            .pending,
        0
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 补不回来的一段记在后面那条上_读收件箱时一起给_确认后跟着删掉() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Mender"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let id = provisioning.id;
    let lobby = lobby();
    let gap = NetworkAgentTimelineGap {
        after_event_id: Some(MatrixEventId::new("$last-seen:matrix.test").expect("事件 ID 有效")),
        reason: NetworkAgentGapReason::TooMany,
    };
    let NetworkAgentInboxChange::Message(mut after_gap) = said("$after-gap:matrix.test", &lobby)
    else {
        unreachable!("said 给的是消息");
    };
    after_gap.gap = Some(gap.clone());
    let NetworkAgentInboxChange::Message(mut unknown_start) = said("$den-gap:matrix.test", &den())
    else {
        unreachable!("said 给的是消息");
    };
    unknown_start.gap = Some(NetworkAgentTimelineGap {
        after_event_id: None,
        reason: NetworkAgentGapReason::TooMany,
    });
    let changes = vec![
        NetworkAgentInboxChange::Message(after_gap),
        said("$next:matrix.test", &lobby),
        NetworkAgentInboxChange::Message(unknown_start),
    ];
    repositories
        .append(&append(id, None, "s1", changes, 10))
        .await
        .expect("写入");

    let page = repositories.pending(id, None, 10).await.expect("读");
    assert_eq!(
        event_ids(&page),
        [
            "$after-gap:matrix.test",
            "$next:matrix.test",
            "$den-gap:matrix.test"
        ]
    );
    assert_eq!(page.entries[0].room_id, lobby);
    assert_eq!(page.entries[0].gaps, [gap]);
    assert!(page.entries[1].gaps.is_empty());
    assert_eq!(page.entries[2].room_id, den());
    assert_eq!(
        page.entries[2].gaps,
        [NetworkAgentTimelineGap {
            after_event_id: None,
            reason: NetworkAgentGapReason::TooMany,
        }],
        "之前最后一条读不出来时只有原因"
    );

    repositories
        .acknowledge(
            id,
            &MatrixEventId::new("$after-gap:matrix.test").expect("事件 ID 有效"),
            Some(&lobby),
        )
        .await
        .expect("确认");
    let page = repositories
        .pending(id, Some(&lobby), 10)
        .await
        .expect("读");
    assert_eq!(event_ids(&page), ["$next:matrix.test"]);
    assert!(
        page.entries[0].gaps.is_empty(),
        "带着它的那条确认了，这一段也就交过了"
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 加入之前解不开的一段挂到这个房间之后第一条消息上_每次加入只说一次() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Latecomer"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let id = provisioning.id;
    let catalog = default_lobby(&database.runtime).await;
    repositories
        .record_room(id, &room_record(catalog, den().as_str(), time(10)))
        .await
        .expect("记房间");
    // 刚进来的那次同步里只有解不开的旧消息，没有能挂的：先记在房间上。没记过的房间不管。
    let mut first = append(id, None, "s1", Vec::new(), 10);
    first.undecryptable_before_join = vec![joined_at(den(), 10), joined_at(lobby(), 10)];
    repositories.append(&first).await.expect("写入");
    assert!(
        repositories
            .pending(id, None, 10)
            .await
            .expect("读")
            .entries
            .is_empty()
    );

    // 之后第一条进收件箱的挂上它；同一条还缺一段补不回来的，加入之前那段排在前面。
    let too_many = NetworkAgentTimelineGap {
        after_event_id: Some(MatrixEventId::new("$mine:matrix.test").expect("事件 ID 有效")),
        reason: NetworkAgentGapReason::TooMany,
    };
    let NetworkAgentInboxChange::Message(mut hello) = said("$hello:matrix.test", &den()) else {
        unreachable!("said 给的是消息");
    };
    hello.gap = Some(too_many.clone());
    let changes = vec![
        authored(
            "$mine:matrix.test",
            &den(),
            MessageId::from_uuid(Uuid::now_v7()),
            "Latecomer",
            true,
            false,
        ),
        NetworkAgentInboxChange::Message(hello),
        said("$again:matrix.test", &den()),
    ];
    repositories
        .append(&append(id, Some("s1"), "s2", changes, 10))
        .await
        .expect("写入");
    let page = repositories.pending(id, None, 10).await.expect("读");
    assert_eq!(
        event_ids(&page),
        ["$hello:matrix.test", "$again:matrix.test"],
        "自己发的不进收件箱，也不挂"
    );
    assert_eq!(page.entries[0].gaps, [before_join_gap(), too_many]);
    assert!(page.entries[1].gaps.is_empty());

    // 说过就不再说：同一次加入又见到（比如存储重建后从头同步）也不挂到新消息上。
    let mut again = append(
        id,
        Some("s2"),
        "s3",
        vec![said("$later:matrix.test", &den())],
        10,
    );
    again.undecryptable_before_join = vec![joined_at(den(), 10)];
    repositories.append(&again).await.expect("写入");
    let page = repositories.pending(id, None, 10).await.expect("读");
    assert_eq!(page.entries[0].gaps.first(), Some(&before_join_gap()));
    assert!(page.entries[2].gaps.is_empty(), "这次加入只说一次");

    // 确认带着它的那条，这一段也跟着交过了。
    repositories
        .acknowledge(
            id,
            &MatrixEventId::new("$hello:matrix.test").expect("事件 ID 有效"),
            None,
        )
        .await
        .expect("确认");
    let page = repositories.pending(id, None, 10).await.expect("读");
    assert!(page.entries.iter().all(|entry| entry.gaps.is_empty()));
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 被移出以后又凭口令进来是更晚的一次加入_不在的那段再说一次() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Returner"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let id = provisioning.id;
    let catalog = default_lobby(&database.runtime).await;
    repositories
        .record_room(id, &room_record(catalog, den().as_str(), time(10)))
        .await
        .expect("记房间");
    let mut first = append(id, None, "s1", vec![said("$hello:matrix.test", &den())], 10);
    first.undecryptable_before_join = vec![joined_at(den(), 10)];
    repositories.append(&first).await.expect("写入");

    // 又进来一次：不在的时候别人说的同样解不开，挂到这之后的第一条上。
    let mut rejoined = append(
        id,
        Some("s1"),
        "s2",
        vec![said("$back:matrix.test", &den())],
        10,
    );
    rejoined.undecryptable_before_join = vec![joined_at(den(), 5_000)];
    repositories.append(&rejoined).await.expect("写入");
    // 更早的那次加入又被同步到，不算新的。
    let mut stale = append(
        id,
        Some("s2"),
        "s3",
        vec![said("$after:matrix.test", &den())],
        10,
    );
    stale.undecryptable_before_join = vec![joined_at(den(), 10)];
    repositories.append(&stale).await.expect("写入");

    let page = repositories.pending(id, None, 10).await.expect("读");
    assert_eq!(
        event_ids(&page),
        [
            "$hello:matrix.test",
            "$back:matrix.test",
            "$after:matrix.test"
        ]
    );
    assert_eq!(page.entries[0].gaps, [before_join_gap()]);
    assert_eq!(page.entries[1].gaps, [before_join_gap()]);
    assert!(page.entries[2].gaps.is_empty(), "更早的那次加入已经说过");
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 按房间往前往后翻_可以只看某个人_只看提到我的() {
    use NetworkAgentHistoryDirection::{After, Before};

    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Reader"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let id = provisioning.id;
    let lobby = lobby();
    let fresh = || MessageId::from_uuid(Uuid::now_v7());
    repositories
        .append(&append(
            id,
            None,
            "s1",
            vec![
                authored("$l1:matrix.test", &lobby, fresh(), "Ranger", false, false),
                authored("$l2:matrix.test", &lobby, fresh(), "Scout", true, false),
                said("$d1:matrix.test", &den()),
                authored("$l3:matrix.test", &lobby, fresh(), "Ranger", false, true),
            ],
            10,
        ))
        .await
        .expect("写入");
    let newest = lobby_page(&repositories, id, Before(None), None, false, 10).await;
    assert_eq!(
        newest,
        ["$l3:matrix.test", "$l2:matrix.test", "$l1:matrix.test"]
    );
    let found = repositories
        .messages_by_id(
            id,
            &[event_ref("$l1:matrix.test"), event_ref("$l3:matrix.test")],
        )
        .await
        .expect("按 ID 取");
    let (first, last) = (found[0].sequence, found[1].sequence);
    assert_eq!(
        lobby_page(&repositories, id, Before(Some(last)), None, false, 1).await,
        ["$l2:matrix.test"]
    );
    assert_eq!(
        lobby_page(&repositories, id, After(first), None, false, 10).await,
        ["$l2:matrix.test", "$l3:matrix.test"]
    );
    assert_eq!(
        lobby_page(&repositories, id, Before(None), None, true, 10).await,
        ["$l3:matrix.test"]
    );
    let ranger = NetworkAgentHistorySender::MatrixUserId("@ranger:matrix.test".to_owned());
    assert_eq!(
        lobby_page(&repositories, id, Before(None), Some(ranger), false, 10).await,
        ["$l3:matrix.test", "$l1:matrix.test"]
    );
    let scout = NetworkAgentHistorySender::NameFolded("scout".to_owned());
    assert_eq!(
        lobby_page(&repositories, id, Before(None), Some(scout), false, 10).await,
        ["$l2:matrix.test"]
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 消息记录每个房间只留最近几条_改了跟着改_撤回就删掉() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Archivist"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let id = provisioning.id;
    let lobby = lobby();
    let (edited, redacted) = (
        MessageId::from_uuid(Uuid::now_v7()),
        MessageId::from_uuid(Uuid::now_v7()),
    );
    let mut first = append(
        id,
        None,
        "s1",
        vec![
            said("$l1:matrix.test", &lobby),
            message_in("$l2:matrix.test", &lobby, redacted, "ranger", "要撤回的"),
            message_in("$l3:matrix.test", &lobby, edited, "ranger", "原来的话"),
            said("$d1:matrix.test", &den()),
        ],
        10,
    );
    first.history_capacity = 2;
    repositories.append(&first).await.expect("写入");
    let refs = [
        event_ref("$l1:matrix.test"),
        event_ref("$l2:matrix.test"),
        event_ref("$l3:matrix.test"),
        event_ref("$d1:matrix.test"),
    ];
    assert_eq!(
        stored_ids(
            &repositories
                .messages_by_id(id, &refs)
                .await
                .expect("按 ID 取")
        ),
        ["$l2:matrix.test", "$l3:matrix.test", "$d1:matrix.test"],
        "大厅只留最近两条，另一个房间不受影响"
    );

    repositories
        .append(&append(
            id,
            Some("s1"),
            "s2",
            vec![
                NetworkAgentInboxChange::Replace {
                    room_id: lobby.clone(),
                    message_id: edited,
                    actor_key: "ranger".to_owned(),
                    patch: json!({"conversation": {"text": "改过的话", "mentions": []}}),
                },
                NetworkAgentInboxChange::Redact {
                    room_id: lobby.clone(),
                    message_id: redacted,
                    actor_key: "ranger".to_owned(),
                },
            ],
            10,
        ))
        .await
        .expect("写入修订");
    let found = repositories
        .messages_by_id(id, &refs)
        .await
        .expect("按 ID 取");
    assert_eq!(stored_ids(&found), ["$l3:matrix.test", "$d1:matrix.test"]);
    assert_eq!(texts(&[found[0].preview.clone()]), ["改过的话"]);
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 发言记录按提交_id_幂等_换内容就冲突_状态只往前走() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Speaker"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let id = provisioning.id;
    let submission = agent_room_domain::ids::MessageSubmissionId::from_uuid(Uuid::now_v7());
    let claim = NetworkAgentSubmissionClaim {
        submission_id: submission,
        kind: NetworkAgentSubmissionKind::Preview,
        fingerprint: [5; 32],
        transaction_id: MatrixTransactionId::new(format!("agent-room-message-{submission}"))
            .expect("事务 ID 有效"),
        claimed_at: time(10),
    };

    let created = repositories.claim(id, &claim).await.expect("占住");
    let NetworkAgentSubmissionClaimOutcome::Created(record) = created else {
        panic!("第一次应当新建");
    };
    assert_eq!(record.state, NetworkAgentSubmissionState::Claimed);
    assert!(matches!(
        repositories.claim(id, &claim).await.expect("重试"),
        NetworkAgentSubmissionClaimOutcome::Existing(_)
    ));
    let conflict = repositories
        .claim(
            id,
            &NetworkAgentSubmissionClaim {
                fingerprint: [6; 32],
                ..claim.clone()
            },
        )
        .await
        .expect_err("换了内容");
    assert_eq!(conflict.kind(), RepositoryErrorKind::Conflict);

    // 发出去没回话，后来同步时按事务 ID 对上。
    assert_eq!(
        repositories
            .mark_submit_unknown(id, submission)
            .await
            .expect("记为不确定")
            .state,
        NetworkAgentSubmissionState::SubmitUnknown
    );
    let event = MatrixEventId::new("$sent:matrix.test").expect("事件 ID 有效");
    let observed = repositories
        .observe_transaction(id, &claim.transaction_id, &event)
        .await
        .expect("对账")
        .expect("找得到");
    assert_eq!(observed.state, NetworkAgentSubmissionState::Accepted);
    assert_eq!(observed.event_id.as_ref(), Some(&event));
    assert_eq!(
        repositories
            .observe_transaction(
                id,
                &MatrixTransactionId::new("agent-room-message-other").expect("事务 ID 有效"),
                &event,
            )
            .await
            .expect("对账"),
        None
    );
    let other_event = MatrixEventId::new("$other:matrix.test").expect("事件 ID 有效");
    assert_eq!(
        repositories
            .mark_accepted(id, submission, &other_event)
            .await
            .expect_err("事件 ID 不能换")
            .kind(),
        RepositoryErrorKind::Conflict
    );
    let bound = repositories.mark_bound(id, submission).await.expect("绑定");
    assert_eq!(bound.state, NetworkAgentSubmissionState::Bound);
    // 已经绑定的不回退。
    assert_eq!(
        repositories
            .mark_accepted(id, submission, &event)
            .await
            .expect("重复确认")
            .state,
        NetworkAgentSubmissionState::Bound
    );
    assert_eq!(
        repositories
            .mark_submit_unknown(id, submission)
            .await
            .expect("不回退")
            .state,
        NetworkAgentSubmissionState::Bound
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 加密房间要用的秘密可以新增与替换_第一次进加密房间的时刻只记一次() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Cipher"), time(0));
    repositories.begin(&provisioning).await.expect("写入");

    for kind in [
        NetworkAgentSecretKind::MatrixStorePassphrase,
        NetworkAgentSecretKind::MatrixRecoveryKey,
        NetworkAgentSecretKind::MessageContentRootKey,
    ] {
        repositories
            .put_secret(provisioning.id, kind, &sealed(7), time(10))
            .await
            .expect("新增");
        assert_eq!(
            repositories
                .find_secret(provisioning.id, kind)
                .await
                .expect("读"),
            Some(sealed(7))
        );
    }
    repositories
        .put_secret(
            provisioning.id,
            NetworkAgentSecretKind::MatrixRecoveryKey,
            &sealed(8),
            time(20),
        )
        .await
        .expect("替换");
    assert_eq!(
        repositories
            .find_secret(provisioning.id, NetworkAgentSecretKind::MatrixRecoveryKey)
            .await
            .expect("读"),
        Some(sealed(8))
    );

    assert_eq!(
        encrypted_since(&repositories, &provisioning.token_digest).await,
        None
    );
    assert_eq!(
        repositories
            .mark_encrypted(provisioning.id, time(30))
            .await
            .expect("记下"),
        time(30)
    );
    assert_eq!(
        repositories
            .mark_encrypted(provisioning.id, time(90))
            .await
            .expect("再记一次"),
        time(30),
        "已经记过的不改"
    );
    assert_eq!(
        encrypted_since(&repositories, &provisioning.token_digest).await,
        Some(time(30))
    );
    assert_eq!(
        repositories
            .mark_encrypted(NetworkAgentId::from_uuid(Uuid::now_v7()), time(30))
            .await
            .expect_err("没有这个网络 Agent")
            .kind(),
        RepositoryErrorKind::NotFound
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 闲置与卡在创建中的被停用_有实例的等着离开房间_记下离开后不再找它() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    // 用远早于其他测试的时间，免得清理碰到并行测试的行。
    let past = |offset: i64| UtcMillis::new(1_500_000_000_000 + offset).expect("时间有效");
    let active = |name: &'static str, last_active: i64| {
        let repositories = &repositories;
        let pool = database.runtime.clone();
        async move {
            let provisioning = provisioning(&unique_name(name), past(0));
            repositories.begin(&provisioning).await.expect("写入");
            let (agent, instance) = seed_agent_instance(
                &pool,
                provisioning.principal.principal.id(),
                provisioning.device.id(),
            )
            .await;
            repositories
                .activate(&NetworkAgentActivation {
                    id: provisioning.id,
                    agent_id: agent,
                    agent_instance_id: instance,
                    matrix_access_token: sealed(3),
                    activated_at: past(0),
                })
                .await
                .expect("生效");
            repositories
                .record_activity(provisioning.id, past(last_active))
                .await
                .expect("记活动");
            provisioning
        }
    };
    let idle = active("Idle", 10).await;
    let recent = active("Recent", 5_000).await;
    let stuck = provisioning(&unique_name("Stuck"), past(0));
    repositories.begin(&stuck).await.expect("写入");
    let fresh = provisioning(&unique_name("Fresh"), past(5_000));
    repositories.begin(&fresh).await.expect("写入");
    let cutoff = NetworkAgentStaleCutoff {
        idle_before: past(1_000),
        provisioning_before: past(1_000),
    };

    let mut disabled = repositories
        .disable_stale(cutoff, past(6_000), 100)
        .await
        .expect("停用闲置与卡住的");
    disabled.sort_by_key(|id| id.as_uuid());
    let mut expected = vec![idle.id, stuck.id];
    expected.sort_by_key(|id| id.as_uuid());
    assert_eq!(disabled, expected);
    assert!(
        repositories
            .disable_stale(cutoff, past(6_000), 100)
            .await
            .expect("再清理一次")
            .is_empty()
    );
    let pending =
        |ids: Vec<NetworkAgentRecord>| ids.into_iter().map(|record| record.id).collect::<Vec<_>>();
    let waiting = pending(repositories.pending_exits(1_000).await.expect("待离开"));
    assert!(waiting.contains(&idle.id), "有实例的要等着离开房间");
    assert!(!waiting.contains(&stuck.id), "没建好实例的从没进过房间");
    assert!(!waiting.contains(&recent.id) && !waiting.contains(&fresh.id));

    repositories
        .disable(recent.id, past(7_000))
        .await
        .expect("自己停用");
    repositories
        .mark_rooms_left(idle.id, past(7_000))
        .await
        .expect("记下已离开");
    repositories
        .mark_rooms_left(fresh.id, past(7_000))
        .await
        .expect("没停用的不记");
    let waiting = pending(repositories.pending_exits(1_000).await.expect("待离开"));
    assert!(!waiting.contains(&idle.id));
    assert!(waiting.contains(&recent.id));
    let fresh_record = repositories
        .find_by_token(&fresh.token_digest)
        .await
        .expect("按令牌找")
        .expect("找得到");
    assert_eq!(fresh_record.status, NetworkAgentStatus::Provisioning);
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 记下离开房间时_它在私人房间的_agent_成员一并记为已移出() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let provisioning = provisioning(&unique_name("Leaver"), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let owner = provisioning.principal.principal.id();
    let (agent, instance) =
        seed_agent_instance(&database.runtime, owner, provisioning.device.id()).await;
    repositories
        .activate(&NetworkAgentActivation {
            id: provisioning.id,
            agent_id: agent,
            agent_instance_id: instance,
            matrix_access_token: sealed(4),
            activated_at: time(0),
        })
        .await
        .expect("生效");
    let joined_room = seed_private_room(&repositories, owner).await;
    let removed_room = seed_private_room(&repositories, owner).await;
    for room in [joined_room, removed_room] {
        repositories
            .admit_agent(
                room,
                agent,
                PrivateRoomPermissions::AGENT_MEMBER,
                PrivateRoomAgentJoinedVia::Code,
                time(5),
            )
            .await
            .expect("凭口令进房间");
    }
    // 早先被房主移出的保持原来的移出时间。
    repositories
        .remove_agent(removed_room, agent, time(8))
        .await
        .expect("房主移出");

    repositories
        .disable(provisioning.id, time(10))
        .await
        .expect("停用");
    let member = |room| {
        let repositories = &repositories;
        async move {
            repositories
                .agent_member(room, agent)
                .await
                .expect("读取成员")
                .expect("有成员记录")
        }
    };
    assert_eq!(
        member(joined_room).await.status,
        PrivateRoomAgentMemberStatus::Joined,
        "停用了但还没离开房间时不动成员"
    );
    repositories
        .mark_rooms_left(provisioning.id, time(20))
        .await
        .expect("记下已离开");
    let left = member(joined_room).await;
    assert_eq!(left.status, PrivateRoomAgentMemberStatus::Removed);
    assert_eq!(left.permissions, PrivateRoomPermissions::NONE);
    assert_eq!(left.status_changed_at, time(20));
    let earlier = member(removed_room).await;
    assert_eq!(earlier.status, PrivateRoomAgentMemberStatus::Removed);
    assert_eq!(earlier.status_changed_at, time(8));
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 离开房间以后删掉秘密与发言限流_抹掉来源摘要_删过的不再改() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let id = activated_agent(&database.runtime, &repositories, "Shredder").await;
    repositories
        .put_secret(
            id,
            NetworkAgentSecretKind::MatrixStorePassphrase,
            &sealed(7),
            time(5),
        )
        .await
        .expect("封存");
    let own = send_buckets(id);
    let stranger = send_buckets(NetworkAgentId::from_uuid(Uuid::now_v7()));
    let policy = RateWindowPolicy {
        window: DurationMillis::new(HOUR).expect("一小时"),
        limit: 10,
    };
    for bucket in own.iter().chain(&stranger) {
        repositories
            .take(bucket, time(5), policy)
            .await
            .expect("计数");
    }

    repositories.disable(id, time(10)).await.expect("停用");
    repositories
        .delete_keys(id, time(15))
        .await
        .expect("还没离开房间");
    assert!(!awaits_key_deletion(&repositories, id).await);
    assert_eq!(
        secret_count(&database.runtime, id).await,
        4,
        "还没离开房间时不删"
    );

    repositories
        .mark_rooms_left(id, time(20))
        .await
        .expect("记下已离开");
    assert!(awaits_key_deletion(&repositories, id).await);
    repositories
        .delete_keys(id, time(30))
        .await
        .expect("删钥匙");

    assert!(!awaits_key_deletion(&repositories, id).await);
    assert_eq!(secret_count(&database.runtime, id).await, 0);
    let all: Vec<String> = own.iter().chain(&stranger).cloned().collect();
    assert_eq!(
        remaining_buckets(&database.runtime, &all).await,
        stranger,
        "只删它自己的发言限流"
    );
    assert_eq!(
        key_deletion(&database.runtime, id).await,
        (Some(time(30)), vec![0; 32])
    );

    repositories
        .delete_keys(id, time(40))
        .await
        .expect("再删一次");
    assert_eq!(
        key_deletion(&database.runtime, id).await.0,
        Some(time(30)),
        "删过的不改"
    );
    database.close().await;
}

async fn seed_private_room(
    repositories: &PostgresRepositories,
    owner: PrincipalId,
) -> RoomCatalogId {
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let catalog = RoomCatalog::new(
        catalog_id,
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: "网络 Agent 私人房间".to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(owner),
            visibility: RoomCatalogVisibility::Private,
            retention_days: Some(30),
            status: RoomCatalogStatus::Active,
        },
    )
    .expect("私人目录有效");
    let instance_id = RoomInstanceId::from_uuid(Uuid::now_v7());
    let instance = RoomInstance::restore(
        instance_id,
        RoomInstanceFields {
            catalog_id,
            matrix_room_id: MatrixRoomReference::new(format!(
                "!leave{}:matrix.test",
                instance_id.as_uuid().simple()
            ))
            .expect("Matrix 房间标识有效"),
            region: None,
            capacity: RoomCapacity::new(8, 16).expect("私人房间容量有效"),
            projected_member_count: 1,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .expect("私人实例有效");
    let snapshot =
        PrivateRoomSnapshot::new(catalog, instance, PrivateRoom::create(catalog_id, owner))
            .expect("私人房间快照有效");
    PrivateRoomStore::create(repositories, &snapshot, time(0))
        .await
        .expect("创建房间");
    catalog_id
}

async fn encrypted_since(
    repositories: &PostgresRepositories,
    digest: &SecretDigest,
) -> Option<UtcMillis> {
    repositories
        .find_by_token(digest)
        .await
        .expect("按令牌找")
        .expect("找得到")
        .encrypted_since
}

/// 取 UUID 末尾的随机部分：开头 8 位是毫秒时间的高位，一分钟内都一样，并行的测试会撞名。
fn unique_name(prefix: &str) -> String {
    format!("{prefix} {}", &Uuid::now_v7().simple().to_string()[24..])
}

async fn default_lobby(pool: &PgPool) -> RoomCatalogId {
    let id: Uuid = sqlx::query_scalar(
        "SELECT id FROM agent_room.room_catalog_entry WHERE slug = 'agent-room-global'",
    )
    .fetch_one(pool)
    .await
    .expect("默认公开大厅由迁移创建");
    RoomCatalogId::from_uuid(id)
}

fn room_record(catalog: RoomCatalogId, room_id: &str, at: UtcMillis) -> NetworkAgentRoomRecord {
    NetworkAgentRoomRecord {
        catalog_id: catalog,
        matrix_room_id: MatrixRoomReference::new(room_id.to_owned()).expect("房间 ID 有效"),
        joined_at: at,
        name: None,
    }
}

/// 这一批里看到它在 `at` 加入这个房间，加入之前有解不开的。
fn joined_at(room: MatrixRoomId, at: i64) -> NetworkAgentBeforeJoinGap {
    NetworkAgentBeforeJoinGap {
        room_id: room,
        joined_at: time(at),
    }
}

/// 交出时带的那一段：加入之前的解不开。
fn before_join_gap() -> NetworkAgentTimelineGap {
    NetworkAgentTimelineGap {
        after_event_id: None,
        reason: NetworkAgentGapReason::UndecryptableBeforeJoin,
    }
}

fn room() -> MatrixRoomId {
    MatrixRoomId::new("!lobby:matrix.test").expect("房间 ID 有效")
}

fn lobby() -> MatrixRoomId {
    MatrixRoomId::new("!lobby:matrix.test").expect("房间 ID 有效")
}

fn den() -> MatrixRoomId {
    MatrixRoomId::new("!den:matrix.test").expect("房间 ID 有效")
}

/// 这个房间里的一条消息，正文就是事件 ID。
fn said(event_id: &str, room_id: &MatrixRoomId) -> NetworkAgentInboxChange {
    message_in(
        event_id,
        room_id,
        MessageId::from_uuid(Uuid::now_v7()),
        "ranger",
        event_id,
    )
}

fn event_ids(page: &NetworkAgentInboxPage) -> Vec<&str> {
    page.entries
        .iter()
        .map(|entry| entry.event_id.as_str())
        .collect()
}

fn message(
    event_id: &str,
    message_id: MessageId,
    actor_key: &str,
    text: &str,
) -> NetworkAgentInboxChange {
    message_in(event_id, &room(), message_id, actor_key, text)
}

fn message_in(
    event_id: &str,
    room_id: &MatrixRoomId,
    message_id: MessageId,
    actor_key: &str,
    text: &str,
) -> NetworkAgentInboxChange {
    NetworkAgentInboxChange::Message(NetworkAgentInboxMessage {
        event_id: MatrixEventId::new(event_id).expect("事件 ID 有效"),
        room_id: room_id.clone(),
        message_id,
        actor_key: actor_key.to_owned(),
        actor: NetworkAgentMessageActor {
            matrix_user_id: format!("@{}:matrix.test", actor_key.to_lowercase()),
            name_folded: actor_key.to_lowercase(),
        },
        from_me: false,
        mentions_me: false,
        gap: None,
        preview: json!({
            "eventId": event_id,
            "messageId": message_id.to_string(),
            "title": text,
            "conversation": {"text": text, "mentions": []},
        }),
    })
}

/// 某人发的一条：`from_me` 是这个网络 Agent 自己发的，`mentions_me` 提到了它。
fn authored(
    event_id: &str,
    room_id: &MatrixRoomId,
    message_id: MessageId,
    actor: &str,
    from_me: bool,
    mentions_me: bool,
) -> NetworkAgentInboxChange {
    let NetworkAgentInboxChange::Message(mut message) =
        message_in(event_id, room_id, message_id, actor, event_id)
    else {
        unreachable!("message_in 给的是消息");
    };
    message.from_me = from_me;
    message.mentions_me = mentions_me;
    NetworkAgentInboxChange::Message(message)
}

fn event_ref(event_id: &str) -> NetworkAgentMessageRef {
    NetworkAgentMessageRef::Event(MatrixEventId::new(event_id).expect("事件 ID 有效"))
}

fn stored_ids(messages: &[NetworkAgentStoredMessage]) -> Vec<&str> {
    messages
        .iter()
        .map(|message| message.event_id.as_str())
        .collect()
}

/// 翻大厅的消息记录，返回事件 ID。
async fn lobby_page(
    repositories: &PostgresRepositories,
    id: NetworkAgentId,
    direction: NetworkAgentHistoryDirection,
    from: Option<NetworkAgentHistorySender>,
    mentions_me: bool,
    limit: u16,
) -> Vec<String> {
    let filter = NetworkAgentHistoryFilter { from, mentions_me };
    stored_ids(
        &repositories
            .room_messages(id, &lobby(), direction, &filter, limit)
            .await
            .expect("翻消息记录"),
    )
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn append(
    id: NetworkAgentId,
    expected: Option<&str>,
    next: &str,
    changes: Vec<NetworkAgentInboxChange>,
    capacity: u32,
) -> NetworkAgentInboxAppend {
    NetworkAgentInboxAppend {
        id,
        expected_sync_token: expected.map(|token| MatrixSyncToken::new(token).expect("位置有效")),
        next_sync_token: MatrixSyncToken::new(next).expect("位置有效"),
        changes,
        received_at: time(40),
        capacity,
        history_capacity: 500,
        undecryptable_before_join: Vec::new(),
    }
}

fn texts(previews: &[Value]) -> Vec<String> {
    previews
        .iter()
        .map(|preview| {
            preview["conversation"]["text"]
                .as_str()
                .expect("有正文")
                .to_owned()
        })
        .collect()
}

/// 加密存储丢了重建时换一台设备：只从记着的那台换走，之后的记录带新设备。
async fn replaces_matrix_device(
    repositories: &PostgresRepositories,
    id: NetworkAgentId,
    instance: AgentInstanceId,
    original: &str,
) {
    let current = AgentMatrixDeviceId::new(original.to_owned()).unwrap();
    let next = AgentMatrixDeviceId::new(format!("{original}_2")).unwrap();
    assert!(
        repositories
            .replace_matrix_device(instance, &current, &next)
            .await
            .expect("换设备")
    );
    assert!(
        !repositories
            .replace_matrix_device(instance, &current, &next)
            .await
            .expect("已经换走的不再换"),
    );
    assert_eq!(
        repositories
            .find(id)
            .await
            .expect("按 ID 找")
            .and_then(|record| record.matrix_device_id),
        Some(next.as_str().to_owned())
    );
}

fn provisioning(name: &str, at: UtcMillis) -> NetworkAgentProvisioning {
    let id = NetworkAgentId::from_uuid(Uuid::now_v7());
    let principal = PrincipalId::from_uuid(Uuid::now_v7());
    let mut device = Device::register(
        DeviceId::from_uuid(Uuid::now_v7()),
        principal,
        "Agent Room 网络 Agent".to_owned(),
        DevicePlatform::Network,
        DevicePublicSigningKey::new(random_bytes().to_vec()).expect("公钥有效"),
        at,
    )
    .expect("设备有效");
    device.verify().expect("可验证");
    NetworkAgentProvisioning {
        id,
        principal: PrincipalRegistration {
            principal: Principal::new(principal),
            oidc_issuer: NETWORK_AGENT_ISSUER.to_owned(),
            oidc_subject: id.to_string(),
            matrix_user_id: format!("@user-{}:matrix.test", id.as_uuid().simple()),
            display_name: name.to_owned(),
            avatar_content_id: None,
            locale: "en".to_owned(),
            registered_at: at,
        },
        device,
        device_label: "Agent Room 网络 Agent".to_owned(),
        token_digest: SecretDigest::from_array(random_bytes()),
        display_name: name.to_owned(),
        source_digest: random_bytes(),
        secrets: vec![
            (NetworkAgentSecretKind::DeviceSigningSeed, sealed(1)),
            (NetworkAgentSecretKind::InstanceSigningSeed, sealed(2)),
        ],
        created_at: at,
    }
}

fn sealed(tag: u8) -> SealedSecret {
    SealedSecret {
        key_version: 1,
        bytes: vec![tag; 60],
    }
}

fn random_bytes() -> [u8; 32] {
    let mut bytes = [0; 32];
    // 两个 UUIDv7 的随机部分足够让测试里的公钥与摘要互不相同。
    bytes[..16].copy_from_slice(Uuid::now_v7().as_bytes());
    bytes[16..].copy_from_slice(Uuid::now_v7().as_bytes());
    bytes
}

/// 网络 Agent 生效前要有它自己的 Agent 与实例；实例挂在网络设备上。
async fn seed_agent_instance(
    pool: &PgPool,
    owner: PrincipalId,
    device: DeviceId,
) -> (AgentId, AgentInstanceId) {
    let agent = AgentId::from_uuid(Uuid::now_v7());
    let binding = Uuid::now_v7();
    let instance = AgentInstanceId::from_uuid(Uuid::now_v7());
    let suffix = agent.as_uuid().simple().to_string();
    let mut transaction = pool.begin().await.expect("事务");
    sqlx::query(
        r"INSERT INTO agent_room.agent (
              id, matrix_user_id, slug, display_name, visibility, lifecycle_state,
              created_at, updated_at
          ) VALUES (
              $1, $2, $3, '网络 Agent', 'private', 'active',
              to_timestamp(1800000000), to_timestamp(1800000000)
          )",
    )
    .bind(agent.as_uuid())
    .bind(format!("@_agent_{suffix}:matrix.test"))
    .bind(format!("host-{suffix}"))
    .execute(&mut *transaction)
    .await
    .expect("Agent 应创建");
    sqlx::query(
        "INSERT INTO agent_room.agent_ownership \
         (principal_id, agent_id, role, granted_by, created_at) \
         VALUES ($1, $2, 'owner', $1, to_timestamp(1800000000))",
    )
    .bind(owner.as_uuid())
    .bind(agent.as_uuid())
    .execute(&mut *transaction)
    .await
    .expect("所有者应写入");
    sqlx::query(
        r"INSERT INTO agent_room.adapter_binding (
              id, agent_id, adapter_type, external_subject_hash,
              capability_version, state, created_at, updated_at
          ) VALUES (
              $1, $2, 'network', NULL, '1.0', 'active',
              to_timestamp(1800000000), to_timestamp(1800000000)
          )",
    )
    .bind(binding)
    .bind(agent.as_uuid())
    .execute(&mut *transaction)
    .await
    .expect("适配器绑定应创建");
    sqlx::query(
        r"INSERT INTO agent_room.agent_instance (
              id, agent_id, device_id, adapter_binding_id, public_signing_key,
              matrix_device_id, status, created_at
          ) VALUES (
              $1, $2, $3, $4, $5, $6, 'connecting', to_timestamp(1800000000)
          )",
    )
    .bind(instance.as_uuid())
    .bind(agent.as_uuid())
    .bind(device.as_uuid())
    .bind(binding)
    .bind(random_bytes().as_slice())
    .bind(format!("AR_{}", instance.as_uuid().simple()))
    .execute(&mut *transaction)
    .await
    .expect("Agent 实例应创建");
    transaction.commit().await.expect("提交");
    (agent, instance)
}

/// 建一个生效的网络 Agent：有实例，停用后要等着替它离开房间。
async fn activated_agent(
    pool: &PgPool,
    repositories: &PostgresRepositories,
    name: &str,
) -> NetworkAgentId {
    let provisioning = provisioning(&unique_name(name), time(0));
    repositories.begin(&provisioning).await.expect("写入");
    let (agent, instance) = seed_agent_instance(
        pool,
        provisioning.principal.principal.id(),
        provisioning.device.id(),
    )
    .await;
    repositories
        .activate(&NetworkAgentActivation {
            id: provisioning.id,
            agent_id: agent,
            agent_instance_id: instance,
            matrix_access_token: sealed(5),
            activated_at: time(0),
        })
        .await
        .expect("生效");
    provisioning.id
}

/// 它的发言限流桶，按名字排好。
fn send_buckets(id: NetworkAgentId) -> Vec<String> {
    let simple = id.as_uuid().simple();
    vec![
        format!("send:day:{simple}"),
        format!("send:minute:{simple}"),
    ]
}

async fn awaits_key_deletion(repositories: &PostgresRepositories, id: NetworkAgentId) -> bool {
    repositories
        .pending_key_deletions(1_000)
        .await
        .expect("待删钥匙")
        .contains(&id)
}

async fn secret_count(pool: &PgPool, id: NetworkAgentId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.network_agent_secret WHERE network_agent_id = $1",
    )
    .bind(id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("数秘密")
}

async fn remaining_buckets(pool: &PgPool, buckets: &[String]) -> Vec<String> {
    sqlx::query_scalar(
        r"SELECT bucket FROM agent_room.network_agent_rate_window
           WHERE bucket = ANY($1)
           ORDER BY bucket",
    )
    .bind(buckets)
    .fetch_all(pool)
    .await
    .expect("读限流桶")
}

/// 删钥匙的时间与来源摘要。
async fn key_deletion(pool: &PgPool, id: NetworkAgentId) -> (Option<UtcMillis>, Vec<u8>) {
    let (deleted_at, digest): (Option<i64>, Vec<u8>) = sqlx::query_as(
        r"SELECT (extract(epoch FROM keys_deleted_at) * 1000)::bigint, source_digest
            FROM agent_room.network_agent
           WHERE id = $1",
    )
    .bind(id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("读网络 Agent");
    (
        deleted_at.map(|at| UtcMillis::new(at).expect("时间有效")),
        digest,
    )
}

fn time(offset: i64) -> UtcMillis {
    UtcMillis::new(1_800_000_000_000 + offset).expect("测试时间有效")
}

fn required_url(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("缺少真实数据库测试配置 {name}"))
}

async fn connect_pool(url: &str) -> PgPool {
    PgPoolOptions::new()
        .min_connections(0)
        .max_connections(5)
        .connect(url)
        .await
        .expect("真实 PostgreSQL 必须可连接")
}
