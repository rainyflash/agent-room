//! 项目房间的认领和绑定（`specs/redesign/project-rooms.md` 的 3P-2a）：两张表的约束；绑着仓库的公开房间
//! 不占每个账户 1 间的名额，转让也不占，不能换分享码；改回私密、归档时解绑，平台拿掉时不动。

use agent_room_application::ports::{PrivateRoomListingRecord, PrivateRoomListingStore};
use agent_room_domain::room_listing::{RoomJoinMode, RoomShareCode};
use sqlx::postgres::PgQueryResult;

use super::{
    listing::{
        Saved, archive, create_listed, create_room, record, save, settings, transfer_room, version,
    },
    *,
};
use agent_room_application::ports::PrivateRoomListingSaveOutcome::{OwnerLimitReached, Stale};

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 一个仓库只绑一间房_大小写不论_名字和文件照规矩_删房间跟着删() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "binding-schema-a").await;
    let other = seed_principal(pool, "binding-schema-b").await;
    let first = create_room(&repositories, owner).await;
    let second = create_room(&repositories, other).await;

    bind(pool, first, owner, "Schema-Owner", "Bound.Repo")
        .await
        .expect("绑得上");
    let lower: (String, String) = sqlx::query_as(
        "SELECT repository_owner_lower, repository_name_lower
           FROM agent_room.room_repository_binding WHERE catalog_entry_id = $1",
    )
    .bind(first.as_uuid())
    .fetch_one(pool)
    .await
    .expect("可读");
    assert_eq!(lower, ("schema-owner".to_owned(), "bound.repo".to_owned()));
    assert_eq!(
        constraint(bind(pool, second, other, "schema-OWNER", "BOUND.repo").await),
        Some("room_repository_binding_repository_unique".to_owned()),
        "大小写不同还是同一个仓库"
    );
    assert_eq!(
        constraint(bind(pool, first, owner, "Schema-Owner", "another").await),
        Some("room_repository_binding_pkey".to_owned()),
        "一间房只绑一个仓库"
    );
    let too_long_owner = "a".repeat(40);
    for (host, repository_owner, name, file, expected) in [
        ("gitlab", "schema-x", "x", "AGENTS.md", "host"),
        ("github", "-schema-x", "x", "AGENTS.md", "owner_format"),
        (
            "github",
            too_long_owner.as_str(),
            "x",
            "AGENTS.md",
            "owner_format",
        ),
        ("github", "schema_x", "x", "AGENTS.md", "owner_format"),
        ("github", "schema-x", "..", "AGENTS.md", "name_format"),
        ("github", "schema-x", "a/b", "AGENTS.md", "name_format"),
        ("github", "schema-x", "x", "CLAUDE.md", "verified_file"),
    ] {
        let result = sqlx::query(
            r"INSERT INTO agent_room.room_repository_binding
                  (catalog_entry_id, host, repository_owner, repository_name, verified_by,
                   verified_at, verified_file)
              VALUES ($1, $2, $3, $4, $5, now(), $6)",
        )
        .bind(second.as_uuid())
        .bind(host)
        .bind(repository_owner)
        .bind(name)
        .bind(other.as_uuid())
        .bind(file)
        .execute(pool)
        .await;
        assert_eq!(
            constraint(result),
            Some(format!("room_repository_binding_{expected}")),
            "{host} {repository_owner}/{name} {file}"
        );
    }
    bind(pool, second, other, "schema-x", "README-only")
        .await
        .expect("合规矩的照样绑得上");

    // 房间删掉时绑定和指着它的认领跟着删（私人房间平时不真删，只能看外键）。
    let cascades: Vec<(String, String)> = sqlx::query_as(
        r"SELECT (SELECT relname FROM pg_class WHERE oid = conrelid)::text,
                 confdeltype::text
            FROM pg_constraint
           WHERE contype = 'f'
             AND confrelid = 'agent_room.room_catalog_entry'::regclass
             AND conrelid IN (
                 'agent_room.repository_claim'::regclass,
                 'agent_room.room_repository_binding'::regclass
             )
           ORDER BY 1",
    )
    .fetch_all(&database.migration)
    .await
    .expect("可读");
    assert_eq!(
        cascades,
        [
            ("repository_claim".to_owned(), "c".to_owned()),
            ("room_repository_binding".to_owned(), "c".to_owned()),
        ]
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 认领每人每个仓库一个_码不重_核对的时间和结果一起记() {
    let database = TestDatabase::connect().await;
    let pool = &database.runtime;
    let owner = seed_principal(pool, "claim-schema-a").await;
    let other = seed_principal(pool, "claim-schema-b").await;

    claim(pool, owner, "Claim-Owner", "Claimed", "Clm001")
        .await
        .expect("认领得上");
    assert_eq!(
        constraint(claim(pool, owner, "claim-owner", "CLAIMED", "Clm002").await),
        Some("repository_claim_principal_repository_unique".to_owned()),
        "同一个人同一个仓库只等着一个"
    );
    claim(pool, other, "claim-owner", "claimed", "Clm002")
        .await
        .expect("别人也能认领同一个仓库，核对时再看谁的码在");
    assert_eq!(
        constraint(claim(pool, other, "claim-owner", "elsewhere", "Clm001").await),
        Some("repository_claim_share_code_unique".to_owned())
    );
    assert_eq!(
        constraint(claim(pool, other, "claim-owner", "elsewhere", "Clm-03").await),
        Some("repository_claim_share_code_format".to_owned())
    );
    assert_eq!(
        constraint(claim(pool, other, "claim_owner", "elsewhere", "Clm003").await),
        Some("repository_claim_owner_format".to_owned())
    );

    let checked = |assignments: &'static str| async move {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE agent_room.repository_claim SET {assignments} WHERE share_code = 'Clm001'"
        )))
        .execute(pool)
        .await
    };
    assert_eq!(
        constraint(checked("last_checked_at = now()").await),
        Some("repository_claim_last_check".to_owned())
    );
    assert_eq!(
        constraint(checked("check_count = -1").await),
        Some("repository_claim_check_count".to_owned())
    );
    assert_eq!(
        constraint(
            checked("last_checked_at = now(), last_check_result = 'Not found', check_count = 1")
                .await
        ),
        Some("repository_claim_last_check_result_format".to_owned())
    );
    checked("last_checked_at = now(), last_check_result = 'not_found', check_count = 1")
        .await
        .expect("记得下核对结果");

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 绑着仓库的公开房间不占名额_改设置和转让照常_不能换分享码() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "binding-quota-a").await;
    let receiver = seed_principal(pool, "binding-quota-b").await;
    let bound = create_room(&repositories, owner).await;
    let free = create_room(&repositories, owner).await;
    let third = create_room(&repositories, owner).await;
    let receivers = create_room(&repositories, receiver).await;

    assert_eq!(
        save(
            &repositories,
            &record(bound, owner, true, "Rpq001", 1),
            None,
            owner
        )
        .await,
        Saved
    );
    bind(pool, bound, owner, "quota-owner", "bound-room")
        .await
        .expect("绑得上");
    assert!(is_bound(&repositories, bound).await);

    // 绑着仓库的那间不算：再公开一间不绑的照样行，之后数名额只数它。
    assert_eq!(
        save(
            &repositories,
            &record(free, owner, true, "Rpq002", 1),
            None,
            owner
        )
        .await,
        Saved
    );
    assert_eq!(unbound_listed(&repositories, owner).await, [free]);
    // 绑着仓库的那间改设置不查名额。
    let bound_changed = PrivateRoomListingRecord {
        settings: settings(true, RoomJoinMode::Request),
        ..record(bound, owner, true, "Rpq001", 2)
    };
    assert_eq!(
        save(&repositories, &bound_changed, Some(version(1)), owner).await,
        Saved
    );
    // 不绑的第二间照旧被拦，新建时就公开的也一样。
    assert_eq!(
        save(
            &repositories,
            &record(third, owner, true, "Rpq003", 1),
            None,
            owner
        )
        .await,
        OwnerLimitReached
    );
    let created = snapshot(RoomCatalogId::from_uuid(Uuid::now_v7()), owner);
    assert_eq!(
        create_listed(
            &repositories,
            &created,
            &record(created.catalog().id(), owner, true, "Rpq004", 1)
        )
        .await,
        OwnerLimitReached
    );

    // 绑着仓库的房间换分享码存不进去，算版本对不上；码没变的照常存。
    let new_code = PrivateRoomListingRecord {
        share_code: RoomShareCode::parse("Rpq005").unwrap(),
        version: version(3),
        ..bound_changed.clone()
    };
    assert_eq!(
        save(&repositories, &new_code, Some(version(2)), owner).await,
        Stale
    );
    assert_eq!(
        PrivateRoomListingStore::listing(&repositories, bound)
            .await
            .expect("可读"),
        Some(bound_changed),
        "码没换"
    );

    // 转给名下已有不绑仓库公开房间的人：绑着仓库的照常转，绑定跟着房间走。
    assert_eq!(
        save(
            &repositories,
            &record(receivers, receiver, true, "Rpq006", 1),
            None,
            receiver
        )
        .await,
        Saved
    );
    let (room, expected) = transfer_room(&repositories, bound, owner, receiver).await;
    assert_eq!(
        PrivateRoomListingStore::save_transferred_room(&repositories, &room, expected, time(3))
            .await
            .expect("写入不报错"),
        Saved
    );
    assert!(is_bound(&repositories, bound).await);
    assert_eq!(unbound_listed(&repositories, receiver).await, [receivers]);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 改回私密和归档时解绑_没存成的不动_平台拿掉的照旧绑着() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "binding-release").await;
    let other = seed_principal(pool, "binding-release-b").await;
    let unlisted = create_room(&repositories, owner).await;
    let archived = create_room(&repositories, owner).await;
    let held = create_room(&repositories, owner).await;
    // 绑着仓库的不占名额，三间都公开得了。
    for (room, code, repository) in [
        (unlisted, "Rpr001", "unlisted-room"),
        (archived, "Rpr002", "archived-room"),
        (held, "Rpr003", "held-room"),
    ] {
        assert_eq!(
            save(
                &repositories,
                &record(room, owner, true, code, 1),
                None,
                owner
            )
            .await,
            Saved
        );
        bind(pool, room, owner, "release-owner", repository)
            .await
            .expect("绑得上");
    }

    // 版本对不上的改回私密什么都没存，也不解绑。
    let private = record(unlisted, owner, false, "Rpr001", 2);
    assert_eq!(
        save(&repositories, &private, Some(version(5)), owner).await,
        Stale
    );
    assert!(is_bound(&repositories, unlisted).await);
    assert_eq!(
        save(&repositories, &private, Some(version(1)), owner).await,
        Saved
    );
    assert!(!is_bound(&repositories, unlisted).await, "改回私密就解绑");
    // 仓库空出来了，别的房间绑得上。
    let elsewhere = create_room(&repositories, other).await;
    bind(pool, elsewhere, other, "release-owner", "unlisted-room")
        .await
        .expect("解绑以后仓库空出来");

    archive(&repositories, archived, owner).await;
    assert!(!is_bound(&repositories, archived).await, "归档就解绑");

    sqlx::query(
        r"INSERT INTO agent_room.public_room_platform_hold
              (catalog_entry_id, held_by, reason, held_at)
          VALUES ($1, $2, '', now())",
    )
    .bind(held.as_uuid())
    .bind(other.as_uuid())
    .execute(pool)
    .await
    .expect("平台拿掉");
    assert!(is_bound(&repositories, held).await, "平台拿掉照旧占着仓库");

    database.close().await;
}

async fn bind(
    pool: &PgPool,
    room: RoomCatalogId,
    verified_by: PrincipalId,
    repository_owner: &str,
    name: &str,
) -> Result<PgQueryResult, sqlx::Error> {
    sqlx::query(
        r"INSERT INTO agent_room.room_repository_binding
              (catalog_entry_id, host, repository_owner, repository_name, verified_by,
               verified_at, verified_file)
          VALUES ($1, 'github', $2, $3, $4, now(), 'AGENTS.md')",
    )
    .bind(room.as_uuid())
    .bind(repository_owner)
    .bind(name)
    .bind(verified_by.as_uuid())
    .execute(pool)
    .await
}

async fn claim(
    pool: &PgPool,
    principal: PrincipalId,
    repository_owner: &str,
    name: &str,
    share_code: &str,
) -> Result<PgQueryResult, sqlx::Error> {
    sqlx::query(
        r"INSERT INTO agent_room.repository_claim
              (id, principal_id, host, repository_owner, repository_name, share_code, claimed_at)
          VALUES ($1, $2, 'github', $3, $4, $5, now())",
    )
    .bind(Uuid::now_v7())
    .bind(principal.as_uuid())
    .bind(repository_owner)
    .bind(name)
    .bind(share_code)
    .execute(pool)
    .await
}

/// 违反的约束名；写成了是 `None`。
fn constraint(result: Result<PgQueryResult, sqlx::Error>) -> Option<String> {
    result
        .err()
        .and_then(|error| error.as_database_error()?.constraint().map(str::to_owned))
}

async fn is_bound(repositories: &PostgresRepositories, room: RoomCatalogId) -> bool {
    PrivateRoomListingStore::repository_bound(repositories, room)
        .await
        .expect("可读")
}

async fn unbound_listed(
    repositories: &PostgresRepositories,
    owner: PrincipalId,
) -> Vec<RoomCatalogId> {
    PrivateRoomListingStore::unbound_listed_rooms_owned_by(repositories, owner)
        .await
        .expect("可读")
}
