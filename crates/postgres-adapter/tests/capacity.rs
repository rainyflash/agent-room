use std::{env, time::Instant};

use agent_room_postgres_adapter::run_migrations;
use serde_json::json;
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgPoolOptions};
use tokio::task::JoinSet;

const AGENT_COUNT: i64 = 10_000;
const INSTANCE_COUNT: i64 = 1_000;
const LOBBY_MEMBER_COUNT: i64 = 250;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "需要由 tools/capacity_database.py 提供隔离的真实 PostgreSQL"]
async fn 设计容量下的目录实例租约与大厅投影达到预算() {
    let migration = connect("AGENT_ROOM_TEST_MIGRATION_DATABASE_URL", 4).await;
    run_migrations(&migration).await.expect("迁移必须成功");
    let runtime = connect("AGENT_ROOM_TEST_RUNTIME_DATABASE_URL", 64).await;

    let seed_started = Instant::now();
    seed_dataset(&migration).await;
    let seed_milliseconds = seed_started.elapsed().as_secs_f64() * 1_000.0;

    let agent_count: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_room.agent")
        .fetch_one(&runtime)
        .await
        .expect("运行时角色可统计 Agent");
    let instance_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.agent_instance WHERE status = 'online'",
    )
    .fetch_one(&runtime)
    .await
    .expect("运行时角色可统计在线实例");
    let lobby_member_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_room.room_membership_projection WHERE matrix_membership = 'join'",
    )
    .fetch_one(&runtime)
    .await
    .expect("运行时角色可统计大厅成员");

    assert_eq!(agent_count, AGENT_COUNT);
    assert_eq!(instance_count, INSTANCE_COUNT);
    assert_eq!(lobby_member_count, LOBBY_MEMBER_COUNT);

    let directory_samples = directory_samples(&runtime).await;
    let directory_p95 = percentile(&directory_samples, 95, 100);
    let (lookup_total, lookup_p95) = concurrent_instance_lookups(&runtime).await;

    let lease_started = Instant::now();
    let renewed = sqlx::query(
        "UPDATE agent_room.agent_instance \
         SET last_seen_at = clock_timestamp(), \
             lease_expires_at = clock_timestamp() + interval '90 seconds' \
         WHERE status = 'online'",
    )
    .execute(&runtime)
    .await
    .expect("批量状态续租成功")
    .rows_affected();
    let lease_milliseconds = lease_started.elapsed().as_secs_f64() * 1_000.0;

    let roster_started = Instant::now();
    let roster = sqlx::query(
        "SELECT agent.display_name, membership.power_level \
         FROM agent_room.room_membership_projection membership \
         JOIN agent_room.agent ON agent.id = membership.agent_id \
         JOIN agent_room.room_instance room ON room.id = membership.room_instance_id \
         WHERE room.matrix_room_id = '!capacity-1:agent-room.test' \
           AND membership.matrix_membership = 'join' \
         ORDER BY agent.slug",
    )
    .fetch_all(&runtime)
    .await
    .expect("250 人大厅投影可读取");
    let roster_milliseconds = roster_started.elapsed().as_secs_f64() * 1_000.0;

    assert_eq!(renewed, u64::try_from(INSTANCE_COUNT).expect("数量可转换"));
    assert_eq!(
        roster.len(),
        usize::try_from(LOBBY_MEMBER_COUNT).expect("数量可转换")
    );
    assert!(
        directory_p95 <= 100.0,
        "目录搜索 P95 超出 100 ms：{directory_p95}"
    );
    assert!(
        lookup_p95 <= 500.0,
        "实例并发查询 P95 超出 500 ms：{lookup_p95}"
    );
    assert!(
        lookup_total <= 10_000.0,
        "1,000 个实例并发查询超过 10 秒：{lookup_total}"
    );
    assert!(
        lease_milliseconds <= 2_000.0,
        "1,000 个实例续租超过 2 秒：{lease_milliseconds}"
    );
    assert!(
        roster_milliseconds <= 500.0,
        "250 人大厅投影超过 500 ms：{roster_milliseconds}"
    );

    let observation = json!({
        "agents": agent_count,
        "onlineInstances": instance_count,
        "lobbyMembers": lobby_member_count,
        "seedMilliseconds": round(seed_milliseconds),
        "directorySearchSamples": directory_samples.len(),
        "directorySearchP95Milliseconds": round(directory_p95),
        "instanceLookupTotalMilliseconds": round(lookup_total),
        "instanceLookupP95Milliseconds": round(lookup_p95),
        "leaseRenewalMilliseconds": round(lease_milliseconds),
        "rosterProjectionMilliseconds": round(roster_milliseconds),
        "nextCapacityThreshold": {
            "agents": 20_000,
            "onlineInstances": 2_000,
            "lobbyMembers": 300
        }
    });
    println!("CAPACITY_OBSERVATION={observation}");

    runtime.close().await;
    migration.close().await;
}

async fn connect(name: &str, maximum_connections: u32) -> PgPool {
    let url = env::var(name).unwrap_or_else(|_| panic!("缺少测试环境变量 {name}"));
    PgPoolOptions::new()
        .max_connections(maximum_connections)
        .connect(&url)
        .await
        .unwrap_or_else(|error| panic!("无法连接容量数据库：{error}"))
}

async fn seed_dataset(pool: &PgPool) {
    let mut transaction = pool.begin().await.expect("容量种子事务可启动");
    seed_principals_and_agents(&mut transaction).await;
    seed_ownership(&mut transaction).await;
    seed_devices_and_bindings(&mut transaction).await;
    seed_online_instances(&mut transaction).await;
    seed_rooms(&mut transaction).await;
    transaction.commit().await.expect("容量种子事务可提交");
}

async fn seed_principals_and_agents(transaction: &mut Transaction<'_, Postgres>) {
    sqlx::query(
        "INSERT INTO agent_room.principal ( \
             id, oidc_issuer, oidc_subject, matrix_user_id, display_name, locale, \
             status, created_at, updated_at \
         ) \
         SELECT uuidv7(), 'https://identity.agent-room.test/realms/capacity', \
                'capacity-principal-' || value, \
                '@capacity-principal-' || value || ':agent-room.test', \
                'Capacity Principal ' || value, 'en', 'active', \
                statement_timestamp(), statement_timestamp() \
         FROM generate_series(1, 10000) AS series(value)",
    )
    .execute(&mut **transaction)
    .await
    .expect("生成 10,000 个主体成功");

    sqlx::query(
        "INSERT INTO agent_room.agent ( \
             id, matrix_user_id, slug, display_name, description, visibility, \
             lifecycle_state, created_at, updated_at \
         ) \
         SELECT uuidv7(), '@capacity-agent-' || value || ':agent-room.test', \
                'capacity-agent-' || lpad(value::text, 5, '0'), \
                'Capacity Agent ' || lpad(value::text, 5, '0'), \
                'Deterministic task 39 dataset', 'public', 'active', \
                statement_timestamp(), statement_timestamp() \
         FROM generate_series(1, 10000) AS series(value)",
    )
    .execute(&mut **transaction)
    .await
    .expect("生成 10,000 个 Agent 成功");
}

async fn seed_ownership(transaction: &mut Transaction<'_, Postgres>) {
    sqlx::query(
        "WITH principals AS ( \
             SELECT id, row_number() OVER (ORDER BY oidc_subject) AS ordinal \
             FROM agent_room.principal \
         ), agents AS ( \
             SELECT id, row_number() OVER (ORDER BY slug) AS ordinal \
             FROM agent_room.agent \
         ) \
         INSERT INTO agent_room.agent_ownership ( \
             principal_id, agent_id, role, granted_by, created_at \
         ) \
         SELECT principals.id, agents.id, 'owner', principals.id, statement_timestamp() \
         FROM principals JOIN agents USING (ordinal)",
    )
    .execute(&mut **transaction)
    .await
    .expect("为 10,000 个 Agent 建立 Owner 成功");
}

async fn seed_devices_and_bindings(transaction: &mut Transaction<'_, Postgres>) {
    sqlx::query(
        "WITH principals AS ( \
             SELECT id, row_number() OVER (ORDER BY oidc_subject) AS ordinal \
             FROM agent_room.principal LIMIT 1000 \
         ) \
         INSERT INTO agent_room.device ( \
             id, principal_id, label, platform, public_signing_key, matrix_device_id, \
             trust_state, last_seen_at, verified_at, created_at \
         ) \
         SELECT uuidv7(), id, 'capacity-device-' || ordinal, 'linux', \
                decode(md5('device-a-' || ordinal) || md5('device-b-' || ordinal), 'hex'), \
                'CAPACITY_DEVICE_' || ordinal, 'verified', statement_timestamp(), \
                statement_timestamp(), statement_timestamp() \
         FROM principals",
    )
    .execute(&mut **transaction)
    .await
    .expect("生成 1,000 个设备成功");

    sqlx::query(
        "WITH agents AS ( \
             SELECT id, row_number() OVER (ORDER BY slug) AS ordinal \
             FROM agent_room.agent LIMIT 1000 \
         ) \
         INSERT INTO agent_room.adapter_binding ( \
             id, agent_id, adapter_type, capability_version, configuration, state, \
             created_at, updated_at \
         ) \
         SELECT uuidv7(), id, 'capacity', '1.0', \
                jsonb_build_object('ordinal', ordinal), 'active', \
                statement_timestamp(), statement_timestamp() \
         FROM agents",
    )
    .execute(&mut **transaction)
    .await
    .expect("生成 1,000 个适配器绑定成功");
}

async fn seed_online_instances(transaction: &mut Transaction<'_, Postgres>) {
    sqlx::query(
        "WITH agents AS ( \
             SELECT id, row_number() OVER (ORDER BY slug) AS ordinal \
             FROM agent_room.agent LIMIT 1000 \
         ), devices AS ( \
             SELECT id, substring(label FROM '[0-9]+$')::bigint AS ordinal \
             FROM agent_room.device WHERE label LIKE 'capacity-device-%' \
         ), bindings AS ( \
             SELECT id, (configuration ->> 'ordinal')::bigint AS ordinal \
             FROM agent_room.adapter_binding WHERE adapter_type = 'capacity' \
         ) \
         INSERT INTO agent_room.agent_instance ( \
             id, agent_id, device_id, adapter_binding_id, public_signing_key, \
             matrix_device_id, status, lease_expires_at, last_seen_at, created_at \
         ) \
         SELECT uuidv7(), agents.id, devices.id, bindings.id, \
                decode(md5('instance-a-' || agents.ordinal) || \
                       md5('instance-b-' || agents.ordinal), 'hex'), \
                'CAPACITY_INSTANCE_' || agents.ordinal, 'online', \
                statement_timestamp() + interval '90 seconds', statement_timestamp(), \
                statement_timestamp() \
         FROM agents \
         JOIN devices USING (ordinal) \
         JOIN bindings USING (ordinal)",
    )
    .execute(&mut **transaction)
    .await
    .expect("生成 1,000 个在线实例成功");
}

async fn seed_rooms(transaction: &mut Transaction<'_, Postgres>) {
    sqlx::query(
        "INSERT INTO agent_room.room_catalog_entry ( \
             id, kind, slug, name, description, language, visibility, status, \
             created_at, updated_at \
         ) VALUES ( \
             uuidv7(), 'public_lobby', 'capacity-lobby', 'Capacity Lobby', \
             'Task 39 capacity room', 'en', 'public', 'active', \
             statement_timestamp(), statement_timestamp() \
         )",
    )
    .execute(&mut **transaction)
    .await
    .expect("生成容量大厅目录成功");

    sqlx::query(
        "INSERT INTO agent_room.room_instance ( \
             id, catalog_entry_id, matrix_room_id, region_hint, soft_capacity, \
             hard_capacity, member_count_projection, allocated_slots, activity_score, \
             state, created_at, updated_at \
         ) \
         SELECT uuidv7(), catalog.id, '!capacity-' || value || ':agent-room.test', \
                'test', 180, 250, CASE WHEN value <= 4 THEN 250 ELSE 0 END, \
                CASE WHEN value <= 4 THEN 250 ELSE 0 END, value, 'active', \
                statement_timestamp(), statement_timestamp() \
         FROM agent_room.room_catalog_entry catalog \
         CROSS JOIN generate_series(1, 5) AS series(value) \
         WHERE catalog.slug = 'capacity-lobby'",
    )
    .execute(&mut **transaction)
    .await
    .expect("生成容量分片成功");

    sqlx::query(
        "WITH agents AS ( \
             SELECT id, row_number() OVER (ORDER BY slug) AS ordinal \
             FROM agent_room.agent LIMIT 250 \
         ), room AS ( \
             SELECT id FROM agent_room.room_instance \
             WHERE matrix_room_id = '!capacity-1:agent-room.test' \
         ) \
         INSERT INTO agent_room.room_membership_projection ( \
             room_instance_id, agent_id, matrix_membership, power_level, \
             last_event_id, projected_at \
         ) \
         SELECT room.id, agents.id, 'join', 0, '$capacity-' || agents.ordinal, \
                statement_timestamp() \
         FROM agents CROSS JOIN room",
    )
    .execute(&mut **transaction)
    .await
    .expect("生成 250 人大厅投影成功");
}

async fn directory_samples(pool: &PgPool) -> Vec<f64> {
    let mut samples = Vec::with_capacity(200);
    for offset in 0..200 {
        let slug = format!("capacity-agent-{:05}", (offset * 47) % 9_950);
        let started = Instant::now();
        let rows = sqlx::query(
            "SELECT id, slug, display_name FROM agent_room.agent \
             WHERE visibility = 'public' AND lifecycle_state = 'active' AND slug >= $1 \
             ORDER BY slug LIMIT 50",
        )
        .bind(slug)
        .fetch_all(pool)
        .await
        .expect("目录范围搜索成功");
        assert_eq!(rows.len(), 50);
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    samples
}

async fn concurrent_instance_lookups(pool: &PgPool) -> (f64, f64) {
    let ids = sqlx::query("SELECT id FROM agent_room.agent_instance ORDER BY matrix_device_id")
        .fetch_all(pool)
        .await
        .expect("读取实例标识成功")
        .into_iter()
        .map(|row| row.get::<uuid::Uuid, _>("id"))
        .collect::<Vec<_>>();
    assert_eq!(
        ids.len(),
        usize::try_from(INSTANCE_COUNT).expect("数量可转换")
    );

    let total_started = Instant::now();
    let mut tasks = JoinSet::new();
    for id in ids {
        let pool = pool.clone();
        tasks.spawn(async move {
            let started = Instant::now();
            let found: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM agent_room.agent_instance WHERE id = $1)",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("并发实例查询成功");
            assert!(found);
            started.elapsed().as_secs_f64() * 1_000.0
        });
    }
    let mut samples = Vec::with_capacity(1_000);
    while let Some(result) = tasks.join_next().await {
        samples.push(result.expect("并发实例查询任务不能崩溃"));
    }
    (
        total_started.elapsed().as_secs_f64() * 1_000.0,
        percentile(&samples, 95, 100),
    )
}

fn percentile(values: &[f64], numerator: usize, denominator: usize) -> f64 {
    assert!(!values.is_empty());
    assert!(numerator > 0 && numerator <= denominator);
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    let rank = ordered
        .len()
        .saturating_mul(numerator)
        .div_ceil(denominator);
    ordered[rank.saturating_sub(1).min(ordered.len() - 1)]
}

fn round(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}
