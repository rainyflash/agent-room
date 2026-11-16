//! 提醒那一轮的候选和房间（specs/redesign/reminders.md 的“怎么算”）。借用自动化测试的种子：一个
//! 人、一间公共大厅，再把大厅改成他加入了的私人房间。

use super::*;
use agent_room_application::ports::{ReminderRecipientRepository, ReminderRoomKind};
use agent_room_domain::network_agents::NETWORK_AGENT_ISSUER;

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 提醒只看还在用的人() {
    let database = TestDatabase::connect().await;
    let reader = seed_automation_fixture(&database.runtime).await;
    let quiet = seed_automation_fixture(&database.runtime).await;
    let suspended = seed_automation_fixture(&database.runtime).await;
    let network = seed_automation_fixture(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    sqlx::query(
        "INSERT INTO agent_room.account_reminder(principal_id,email_enabled,email_changed_at,last_email_at,covered_until,updated_at)
         VALUES($1,false,clock_timestamp(),NULL,NULL,clock_timestamp()),
               ($2,true,clock_timestamp(),to_timestamp(1790000000),to_timestamp(1790000000),clock_timestamp())",
    )
    .bind(quiet.principal.as_uuid())
    .bind(reader.principal.as_uuid())
    .execute(&database.runtime)
    .await
    .expect("提醒设置");
    sqlx::query("UPDATE agent_room.principal SET status='suspended' WHERE id=$1")
        .bind(suspended.principal.as_uuid())
        .execute(&database.runtime)
        .await
        .expect("停用");
    sqlx::query("UPDATE agent_room.principal SET oidc_issuer=$2 WHERE id=$1")
        .bind(network.principal.as_uuid())
        .bind(NETWORK_AGENT_ISSUER)
        .execute(&database.runtime)
        .await
        .expect("网络 Agent 的主体");

    let candidates = repositories
        .reminder_candidates(None, 1_000)
        .await
        .expect("候选");
    let ours: Vec<_> = candidates
        .iter()
        .filter(|candidate| {
            [
                reader.principal,
                quiet.principal,
                suspended.principal,
                network.principal,
            ]
            .contains(&candidate.principal_id)
        })
        .collect();
    assert_eq!(ours.len(), 1, "关了提醒的、停用的、网络 Agent 都不看");
    assert_eq!(ours[0].principal_id, reader.principal);
    assert_eq!(
        ours[0].last_email_at.map(UtcMillis::value),
        Some(1_790_000_000_000)
    );
    assert!(
        repositories
            .reminder_candidates(Some(reader.principal), 1_000)
            .await
            .expect("候选")
            .iter()
            .all(|candidate| candidate.principal_id > reader.principal),
        "按账户 ID 往后翻"
    );
    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 提醒只看他加入了的私人房间_归档了的不算() {
    let database = TestDatabase::connect().await;
    let reader = seed_automation_fixture(&database.runtime).await;
    let outsider = seed_automation_fixture(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    assert!(
        repositories
            .reminder_rooms(reader.principal)
            .await
            .expect("房间")
            .is_empty(),
        "公共大厅不算"
    );
    let mut transaction = database.runtime.begin().await.expect("改成私人房间");
    sqlx::query("UPDATE agent_room.room_catalog_entry SET kind='private_room',visibility='private',owner_principal_id=$2 WHERE id=$1")
        .bind(reader.catalog.as_uuid()).bind(reader.principal.as_uuid()).execute(&mut *transaction).await.expect("私人房间");
    sqlx::query("INSERT INTO agent_room.private_room_state(catalog_entry_id,room_instance_id,created_at,updated_at) SELECT $1,id,clock_timestamp(),clock_timestamp() FROM agent_room.room_instance WHERE catalog_entry_id=$1")
        .bind(reader.catalog.as_uuid()).execute(&mut *transaction).await.expect("房间状态");
    sqlx::query("INSERT INTO agent_room.private_room_membership(catalog_entry_id,principal_id,membership_status,permission_bits,created_at,status_changed_at) VALUES($1,$2,'joined',31,clock_timestamp(),to_timestamp(1790000100))")
        .bind(reader.catalog.as_uuid()).bind(reader.principal.as_uuid()).execute(&mut *transaction).await.expect("加入");
    transaction.commit().await.expect("私人房间完整");

    let rooms = repositories
        .reminder_rooms(reader.principal)
        .await
        .expect("房间");
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0].catalog_id, reader.catalog);
    assert_eq!(rooms[0].matrix_room_id, reader.matrix_room);
    assert_eq!(rooms[0].kind, ReminderRoomKind::Room);
    assert!(rooms[0].end_to_end_encrypted);
    assert_eq!(rooms[0].joined_at.value(), 1_790_000_100_000);
    assert!(
        repositories
            .reminder_rooms(outsider.principal)
            .await
            .expect("房间")
            .is_empty(),
        "没加入的人不算"
    );

    let mut transaction = database.runtime.begin().await.expect("归档");
    sqlx::query("UPDATE agent_room.room_instance SET state='archived' WHERE catalog_entry_id=$1")
        .bind(reader.catalog.as_uuid())
        .execute(&mut *transaction)
        .await
        .expect("归档实例");
    sqlx::query("UPDATE agent_room.room_catalog_entry SET status='archived' WHERE id=$1")
        .bind(reader.catalog.as_uuid())
        .execute(&mut *transaction)
        .await
        .expect("归档目录");
    transaction.commit().await.expect("归档完整");
    assert!(
        repositories
            .reminder_rooms(reader.principal)
            .await
            .expect("房间")
            .is_empty(),
        "归档了的不算"
    );
    database.close().await;
}
