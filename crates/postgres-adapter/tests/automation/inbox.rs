use super::*;
use agent_room_application::ports::PersonalInboxRepository;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn inbox_routing_respects_membership_and_catalog_lifecycle() {
    let database = TestDatabase::connect().await;
    let owner = seed_automation_fixture(&database.runtime).await;
    let stranger = seed_automation_fixture(&database.runtime).await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    assert!(
        repositories
            .index(stranger.principal)
            .await
            .expect("public metadata")
            .rooms
            .iter()
            .any(|room| room.room_id == owner.matrix_room.as_str())
    );
    let mut transaction = database.runtime.begin().await.expect("private conversion");
    sqlx::query("UPDATE agent_room.room_catalog_entry SET kind='private_room',visibility='private',owner_principal_id=$2 WHERE id=$1")
        .bind(owner.catalog.as_uuid()).bind(owner.principal.as_uuid()).execute(&mut *transaction).await.expect("private catalog");
    sqlx::query("INSERT INTO agent_room.private_room_state(catalog_entry_id,room_instance_id,created_at,updated_at) SELECT $1,id,clock_timestamp(),clock_timestamp() FROM agent_room.room_instance WHERE catalog_entry_id=$1")
        .bind(owner.catalog.as_uuid()).execute(&mut *transaction).await.expect("private state");
    sqlx::query("INSERT INTO agent_room.private_room_membership(catalog_entry_id,principal_id,membership_status,permission_bits,created_at,status_changed_at) VALUES($1,$2,'joined',31,clock_timestamp(),clock_timestamp())")
        .bind(owner.catalog.as_uuid()).bind(owner.principal.as_uuid()).execute(&mut *transaction).await.expect("owner membership");
    transaction.commit().await.expect("private integrity");
    assert!(
        repositories
            .index(owner.principal)
            .await
            .expect("owner inbox")
            .rooms
            .iter()
            .any(|room| room.room_id == owner.matrix_room.as_str())
    );
    assert!(
        !repositories
            .index(stranger.principal)
            .await
            .expect("stranger inbox")
            .rooms
            .iter()
            .any(|room| room.room_id == owner.matrix_room.as_str())
    );
    let mut transaction = database.runtime.begin().await.expect("archive transaction");
    sqlx::query("UPDATE agent_room.room_instance SET state='archived' WHERE catalog_entry_id=$1")
        .bind(owner.catalog.as_uuid())
        .execute(&mut *transaction)
        .await
        .expect("archive instance");
    sqlx::query("UPDATE agent_room.room_catalog_entry SET status='archived' WHERE id=$1")
        .bind(owner.catalog.as_uuid())
        .execute(&mut *transaction)
        .await
        .expect("archive");
    transaction.commit().await.expect("archive integrity");
    assert!(
        !repositories
            .index(owner.principal)
            .await
            .expect("archived inbox")
            .rooms
            .iter()
            .any(|room| room.room_id == owner.matrix_room.as_str())
    );
    database.close().await;
}
