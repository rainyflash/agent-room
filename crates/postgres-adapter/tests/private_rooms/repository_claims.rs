//! 仓库认领和绑定的仓储（`specs/redesign/project-rooms.md` 的 3P-2b）：同一个人同一个仓库回原来那个、
//! 最多几个在等、码不和认领与公开房间重；开始核对的时间戳挡住同时核对；该核对的和作废的；绑上、挪走、
//! 解绑；认上以后建项目房间在同一个事务里；导出里有认领和绑定。
//!
//! 库是这个测试程序里各个用例共用的：码、仓库名各用各的，按时间作废的只用 1970 年的认领。

use agent_room_application::ports::{
    AccountDeletionRepository, BindRepository, BindRepositoryOutcome, DueClaimQuery,
    PrivateRoomListingStore, ProjectRoomSaveOutcome, ProjectRoomStore, RepositoryBindingRecord,
    RepositoryCheckRecord, RepositoryClaimCheckStart, RepositoryClaimInsertOutcome,
    RepositoryClaimRecord, RoomLiveViewRecord, RoomLiveViewStore, UnbindRepositoryOutcome,
};
use agent_room_domain::{
    ids::RepositoryClaimId,
    project_rooms::{ClaimCheckResult, GithubRepository, RepositoryFile},
    room_listing::RoomShareCode,
};

use super::{
    listing::{Saved, create_room, record, save, version},
    *,
};

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 认领回原来那个_最多几个在等_码不重() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "claims-insert-a").await;
    let other = seed_principal(pool, "claims-insert-b").await;

    let first = claim_record(owner, "claims-a/One", "Rci001", None, time(0));
    assert_eq!(
        insert(&repositories, &first, 3).await,
        RepositoryClaimInsertOutcome::Inserted(first.clone())
    );
    let again = claim_record(owner, "CLAIMS-A/one", "Rci009", None, time(5));
    assert_eq!(
        insert(&repositories, &again, 3).await,
        RepositoryClaimInsertOutcome::Existing(first.clone()),
        "大小写不同也是同一个仓库，回原来那个"
    );
    for (repository, code) in [("claims-a/two", "Rci002"), ("claims-a/three", "Rci003")] {
        let claim = claim_record(owner, repository, code, None, time(1));
        assert!(matches!(
            insert(&repositories, &claim, 3).await,
            RepositoryClaimInsertOutcome::Inserted(_)
        ));
    }
    let fourth = claim_record(owner, "claims-a/four", "Rci004", None, time(2));
    assert_eq!(
        insert(&repositories, &fourth, 3).await,
        RepositoryClaimInsertOutcome::TooMany
    );
    let same_code = claim_record(other, "claims-a/four", "Rci001", None, time(2));
    assert_eq!(
        insert(&repositories, &same_code, 3).await,
        RepositoryClaimInsertOutcome::ShareCodeTaken
    );
    // 公开房间已经在用的码也不行。
    let listed = create_room(&repositories, other).await;
    assert_eq!(
        save(
            &repositories,
            &record(listed, other, true, "Rci050", 1),
            None,
            other
        )
        .await,
        Saved
    );
    let listed_code = claim_record(other, "claims-a/four", "Rci050", None, time(2));
    assert_eq!(
        insert(&repositories, &listed_code, 3).await,
        RepositoryClaimInsertOutcome::ShareCodeTaken
    );

    let mine = ProjectRoomStore::claims_of(&repositories, owner)
        .await
        .expect("可读");
    assert_eq!(mine.len(), 3);
    assert_eq!(mine[0], first, "最早的在前");
    assert!(
        !ProjectRoomStore::delete_claim(&repositories, first.id, other)
            .await
            .expect("可删"),
        "别人撤不掉"
    );
    assert!(
        ProjectRoomStore::delete_claim(&repositories, first.id, owner)
            .await
            .expect("可删")
    );
    assert!(matches!(
        insert(&repositories, &fourth, 3).await,
        RepositoryClaimInsertOutcome::Inserted(_)
    ));

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 拿已有的房间认领_换一间房就换成新的_同一间房只留最新的() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "claims-target").await;
    let room = create_room(&repositories, owner).await;
    let another = create_room(&repositories, owner).await;

    let to_room = claim_record(owner, "claims-t/one", "Rct001", Some(room), time(0));
    insert(&repositories, &to_room, 3).await;
    let to_another = claim_record(owner, "claims-t/one", "Rct002", Some(another), time(1));
    assert!(matches!(
        insert(&repositories, &to_another, 3).await,
        RepositoryClaimInsertOutcome::Inserted(_)
    ));
    let other_repository = claim_record(owner, "claims-t/two", "Rct003", Some(another), time(2));
    assert!(matches!(
        insert(&repositories, &other_repository, 3).await,
        RepositoryClaimInsertOutcome::Inserted(_)
    ));
    let mine = ProjectRoomStore::claims_of(&repositories, owner)
        .await
        .expect("可读");
    assert_eq!(
        mine,
        [other_repository],
        "同一个仓库换了房间、同一间房换了仓库，都只留新的"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 开始核对挡住同时核对_结果只记给自己那一次() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "claims-check").await;
    let young = claim_record(owner, "claims-c/young", "Rcc002", None, time(0));
    insert(&repositories, &young, 3).await;

    let now = time(10 * MINUTE);
    let started = start(&repositories, young.id, now, epoch(now.value() - MINUTE)).await;
    let RepositoryClaimCheckStart::Started(started) = started else {
        panic!("第一次核对记得下：{started:?}");
    };
    assert_eq!(
        started.last_check,
        Some(RepositoryCheckRecord {
            at: now,
            result: ClaimCheckResult::Checking
        })
    );
    assert_eq!(started.check_count, 1);
    let later = epoch(now.value() + 20_000);
    assert_eq!(
        start(
            &repositories,
            young.id,
            later,
            epoch(later.value() - MINUTE)
        )
        .await,
        RepositoryClaimCheckStart::TooSoon {
            retry_at: epoch(now.value() + MINUTE)
        }
    );
    // 别处后来又核对过，旧的那次的结果不记。
    finish(
        &repositories,
        young.id,
        epoch(now.value() - 1),
        ClaimCheckResult::NotInRepository,
    )
    .await;
    assert_eq!(
        last_check(&repositories, young.id).await,
        Some((now, ClaimCheckResult::Checking))
    );
    finish(
        &repositories,
        young.id,
        now,
        ClaimCheckResult::NotInRepository,
    )
    .await;
    assert_eq!(
        last_check(&repositories, young.id).await,
        Some((now, ClaimCheckResult::NotInRepository))
    );
    let again = epoch(now.value() + MINUTE);
    let RepositoryClaimCheckStart::Started(again) = start(
        &repositories,
        young.id,
        again,
        epoch(again.value() - MINUTE),
    )
    .await
    else {
        panic!("过了一分钟又能核对");
    };
    assert_eq!(
        again.last_check.map(|check| check.result),
        Some(ClaimCheckResult::NotInRepository),
        "结果先留着上次的，读完再换"
    );
    assert_eq!(again.check_count, 2);
    assert_eq!(
        start(
            &repositories,
            RepositoryClaimId::from_uuid(Uuid::now_v7()),
            now,
            now
        )
        .await,
        RepositoryClaimCheckStart::Gone
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 该核对的按上次核对的时间挑_满了时候的作废() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "claims-due").await;
    // 1970 年的认领：作废那一步只会删到这几个。
    let old = claim_record(owner, "claims-d/old", "Rcd001", None, epoch(1_000));
    let young = claim_record(owner, "claims-d/young", "Rcd002", None, epoch(10 * HOUR));
    for claim in [&old, &young] {
        insert(&repositories, claim, 3).await;
    }
    let checked = epoch(10 * HOUR + 11 * MINUTE);
    assert!(matches!(
        start(&repositories, young.id, checked, checked).await,
        RepositoryClaimCheckStart::Started(_)
    ));

    // 头 24 小时里的刚核对过不到 5 分钟，不该核对；过了 24 小时的从没核对过，该核对。
    let due = ProjectRoomStore::due_claims(
        &repositories,
        DueClaimQuery {
            now: epoch(DAY + 2 * HOUR),
            young_since: epoch(2 * HOUR),
            young_checked_before: epoch(DAY + 2 * HOUR - 5 * MINUTE),
            old_checked_before: epoch(DAY + HOUR),
            limit: 1_000,
        },
    )
    .await
    .expect("可读");
    assert!(due.contains(&old.id));
    assert!(due.contains(&young.id), "过了五分钟");
    let due = ProjectRoomStore::due_claims(
        &repositories,
        DueClaimQuery {
            now: epoch(10 * HOUR + 13 * MINUTE),
            young_since: epoch(0),
            young_checked_before: epoch(10 * HOUR + 8 * MINUTE),
            old_checked_before: epoch(0),
            limit: 1_000,
        },
    )
    .await
    .expect("可读");
    assert!(due.contains(&old.id), "从没核对过的按认领的时间算");
    assert!(!due.contains(&young.id), "刚核对过不到五分钟");

    let expired = ProjectRoomStore::expire_claims(&repositories, epoch(2_000))
        .await
        .expect("可删");
    assert_eq!(expired, 1);
    assert_eq!(find(&repositories, old.id).await, None);
    assert!(find(&repositories, young.id).await.is_some());

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 认上以后绑到已有的房间_仓库被占着不绑_挪走原来的_平台拿掉的照旧占着() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let first_owner = seed_principal(pool, "claims-bind-a").await;
    let second_owner = seed_principal(pool, "claims-bind-b").await;
    let first = listed_room(&repositories, first_owner, "Rcb001").await;
    let second = listed_room(&repositories, second_owner, "Rcb002").await;
    let unlisted = create_room(&repositories, second_owner).await;
    let repository = GithubRepository::parse("claims-b/Bound").unwrap();

    let first_claim = claim_record(
        first_owner,
        "claims-b/bound",
        "Rcb001",
        Some(first),
        time(0),
    );
    insert(&repositories, &first_claim, 3).await;
    assert_eq!(
        bind(&repositories, &bind_request(&first_claim, first, None)).await,
        BindRepositoryOutcome::Bound
    );
    assert_eq!(find(&repositories, first_claim.id).await, None, "认上就删");
    let bound = ProjectRoomStore::bound_room(&repositories, &repository)
        .await
        .expect("可读")
        .expect("绑着");
    assert_eq!(bound.binding.catalog_id, first);
    assert_eq!(bound.share_code, Some(share_code("Rcb001")));
    assert!(!bound.held);
    assert_eq!(
        bind(&repositories, &bind_request(&first_claim, first, None)).await,
        BindRepositoryOutcome::ClaimGone
    );

    // 第二个人的认领：仓库被占着、不挪就绑不上，认领留着；没公开的房间绑不上。
    let second_claim = claim_record(
        second_owner,
        "claims-b/bound",
        "Rcb002",
        Some(second),
        time(1),
    );
    insert(&repositories, &second_claim, 3).await;
    assert_eq!(
        bind(&repositories, &bind_request(&second_claim, second, None)).await,
        BindRepositoryOutcome::RepositoryTaken
    );
    assert!(find(&repositories, second_claim.id).await.is_some());
    assert_eq!(
        bind(
            &repositories,
            &bind_request(&second_claim, unlisted, Some(first))
        )
        .await,
        BindRepositoryOutcome::RoomNotEligible
    );
    assert_eq!(
        bind(
            &repositories,
            &bind_request(&second_claim, second, Some(first))
        )
        .await,
        BindRepositoryOutcome::Bound,
        "原来那间房的链接不在了，绑定挪过来"
    );
    assert_eq!(binding(&repositories, first).await, None);
    let moved = binding(&repositories, second).await.expect("挪过来了");
    assert_eq!(moved.verified_by, second_owner);
    let candidate = ProjectRoomStore::project_room_candidate(&repositories, second)
        .await
        .expect("可读")
        .expect("有这间房");
    assert_eq!(
        candidate.bound.map(|bound| bound.full_name()),
        Some("claims-b/bound".to_owned())
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 房主再核对一次记下这次_平台拿掉的房间照旧占着仓库() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "claims-refresh").await;
    let operator = seed_principal(pool, "claims-refresh-b").await;
    let room = listed_room(&repositories, owner, "Rcr001").await;
    let claim = claim_record(owner, "claims-r/bound", "Rcr001", Some(room), time(0));
    insert(&repositories, &claim, 3).await;
    assert_eq!(
        bind(&repositories, &bind_request(&claim, room, None)).await,
        BindRepositoryOutcome::Bound
    );
    let bound = binding(&repositories, room).await.expect("绑着");

    // 记下谁、什么时候、哪个文件；仓库不对不动。
    let refreshed = RepositoryBindingRecord {
        verified_by: owner,
        verified_at: time(50),
        verified_file: RepositoryFile::ReadmeMd,
        ..bound
    };
    assert!(
        ProjectRoomStore::refresh_binding(&repositories, &refreshed)
            .await
            .expect("可写")
    );
    assert_eq!(binding(&repositories, room).await, Some(refreshed.clone()));
    let elsewhere = RepositoryBindingRecord {
        repository: GithubRepository::parse("claims-r/elsewhere").unwrap(),
        ..refreshed
    };
    assert!(
        !ProjectRoomStore::refresh_binding(&repositories, &elsewhere)
            .await
            .expect("可写")
    );

    sqlx::query(
        r"INSERT INTO agent_room.public_room_platform_hold
              (catalog_entry_id, held_by, reason, held_at)
          VALUES ($1, $2, '', now())",
    )
    .bind(room.as_uuid())
    .bind(operator.as_uuid())
    .execute(pool)
    .await
    .expect("平台拿掉");
    let held = ProjectRoomStore::bound_room(&repositories, &claim.repository)
        .await
        .expect("可读")
        .expect("照旧绑着");
    assert!(held.held);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 房主解绑_名下另有一间不绑仓库的公开房间时不解() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "claims-unbind").await;
    let stranger = seed_principal(pool, "claims-unbind-b").await;
    let bound = listed_room(&repositories, owner, "Rcu001").await;
    let claim = claim_record(owner, "claims-u/bound", "Rcu001", Some(bound), time(0));
    insert(&repositories, &claim, 3).await;
    assert_eq!(
        bind(&repositories, &bind_request(&claim, bound, None)).await,
        BindRepositoryOutcome::Bound
    );
    let free = listed_room(&repositories, owner, "Rcu002").await;

    assert_eq!(
        unbind(&repositories, bound, stranger).await,
        UnbindRepositoryOutcome::NotOwner
    );
    assert_eq!(
        unbind(&repositories, bound, owner).await,
        UnbindRepositoryOutcome::LimitReached
    );
    assert!(binding(&repositories, bound).await.is_some());
    assert_eq!(
        save(
            &repositories,
            &record(free, owner, false, "Rcu002", 2),
            Some(version(1)),
            owner
        )
        .await,
        Saved
    );
    assert_eq!(
        unbind(&repositories, bound, owner).await,
        UnbindRepositoryOutcome::Unbound
    );
    assert_eq!(binding(&repositories, bound).await, None);
    assert_eq!(
        unbind(&repositories, bound, owner).await,
        UnbindRepositoryOutcome::NotBound
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 认上以后建项目房间_房间公开不加密_绑上删认领一个事务_不占名额_导出里都有() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "claims-create").await;
    // 他已经有一间不绑仓库的公开房间：项目房间照样建得了。
    listed_room(&repositories, owner, "Rcp009").await;
    let claim = claim_record(owner, "claims-p/Created", "Rcp001", None, time(0));
    insert(&repositories, &claim, 3).await;
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let created = snapshot(catalog_id, owner);
    let listing = record(catalog_id, owner, true, "Rcp001", 1);
    let request = bind_request(&claim, catalog_id, None);

    assert_eq!(
        PrivateRoomListingStore::create_project_room(
            &repositories,
            &created,
            &listing,
            &request,
            time(3)
        )
        .await
        .expect("写入不报错"),
        ProjectRoomSaveOutcome::Saved
    );
    assert_eq!(find(&repositories, claim.id).await, None);
    assert_eq!(
        PrivateRoomListingStore::listing(&repositories, catalog_id)
            .await
            .expect("可读"),
        Some(listing.clone())
    );
    assert_eq!(
        binding(&repositories, catalog_id)
            .await
            .map(|binding| binding.verified_file),
        Some(RepositoryFile::AgentsMd)
    );
    let encrypted: bool = sqlx::query_scalar(
        "SELECT end_to_end_encrypted FROM agent_room.room_catalog_entry WHERE id = $1",
    )
    .bind(catalog_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("可读");
    assert!(!encrypted, "项目房间不加密");
    assert_eq!(
        RoomLiveViewStore::live_view_facts(&repositories, catalog_id)
            .await
            .expect("可读")
            .expect("有房间")
            .live_view,
        Some(RoomLiveViewRecord {
            enabled_by: owner,
            enabled_at: time(3),
        }),
        "项目房间建好就放实况，从建房那一刻起"
    );

    let another = RoomCatalogId::from_uuid(Uuid::now_v7());
    assert_eq!(
        PrivateRoomListingStore::create_project_room(
            &repositories,
            &snapshot(another, owner),
            &record(another, owner, true, "Rcp001", 1),
            &bind_request(&claim, another, None),
            time(4)
        )
        .await
        .expect("写入不报错"),
        ProjectRoomSaveOutcome::ClaimGone,
        "认领没了就不建"
    );
    assert!(
        PrivateRoomStore::find_by_catalog(&repositories, another)
            .await
            .expect("可读")
            .is_none(),
        "整个事务都没存"
    );

    let pending = claim_record(owner, "claims-p/pending", "Rcp002", None, time(5));
    insert(&repositories, &pending, 3).await;
    let exported = AccountDeletionRepository::export(&repositories, owner, time(6))
        .await
        .expect("账户导出应成功")
        .expect("活动账户必须存在");
    assert_eq!(
        exported.data["repositoryClaims"][0]["repository"],
        "claims-p/pending"
    );
    assert_eq!(exported.data["repositoryClaims"][0]["shareCode"], "Rcp002");
    assert_eq!(
        exported.data["repositoryBindings"][0]["repository"],
        "claims-p/Created"
    );
    assert_eq!(
        exported.data["repositoryBindings"][0]["roomId"],
        catalog_id.as_uuid().to_string()
    );

    database.close().await;
}

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// 1970 年的某一刻：作废那一步只会删到这些。
fn epoch(millis: i64) -> UtcMillis {
    UtcMillis::new(millis).unwrap()
}

fn share_code(value: &str) -> RoomShareCode {
    RoomShareCode::parse(value).unwrap()
}

fn claim_record(
    principal_id: PrincipalId,
    repository: &str,
    code: &str,
    target: Option<RoomCatalogId>,
    claimed_at: UtcMillis,
) -> RepositoryClaimRecord {
    RepositoryClaimRecord {
        id: RepositoryClaimId::from_uuid(Uuid::now_v7()),
        principal_id,
        repository: GithubRepository::parse(repository).unwrap(),
        share_code: share_code(code),
        target,
        claimed_at,
        last_check: None,
        check_count: 0,
    }
}

fn bind_request(
    claim: &RepositoryClaimRecord,
    catalog_id: RoomCatalogId,
    replacing: Option<RoomCatalogId>,
) -> BindRepository {
    BindRepository {
        claim_id: claim.id,
        catalog_id,
        repository: claim.repository.clone(),
        verified_by: claim.principal_id,
        verified_at: time(10),
        verified_file: RepositoryFile::AgentsMd,
        share_code: claim.share_code.clone(),
        replacing,
    }
}

async fn listed_room(
    repositories: &PostgresRepositories,
    owner: PrincipalId,
    code: &str,
) -> RoomCatalogId {
    let room = create_room(repositories, owner).await;
    assert_eq!(
        save(
            repositories,
            &record(room, owner, true, code, 1),
            None,
            owner
        )
        .await,
        Saved
    );
    room
}

async fn insert(
    repositories: &PostgresRepositories,
    claim: &RepositoryClaimRecord,
    max_pending: u32,
) -> RepositoryClaimInsertOutcome {
    ProjectRoomStore::insert_claim(repositories, claim, max_pending)
        .await
        .expect("写入不报错")
}

async fn find(
    repositories: &PostgresRepositories,
    claim_id: RepositoryClaimId,
) -> Option<RepositoryClaimRecord> {
    ProjectRoomStore::claim(repositories, claim_id)
        .await
        .expect("可读")
}

async fn last_check(
    repositories: &PostgresRepositories,
    claim_id: RepositoryClaimId,
) -> Option<(UtcMillis, ClaimCheckResult)> {
    find(repositories, claim_id)
        .await
        .and_then(|claim| claim.last_check)
        .map(|check| (check.at, check.result))
}

async fn start(
    repositories: &PostgresRepositories,
    claim_id: RepositoryClaimId,
    now: UtcMillis,
    checked_before: UtcMillis,
) -> RepositoryClaimCheckStart {
    ProjectRoomStore::start_check(repositories, claim_id, now, checked_before)
        .await
        .expect("可写")
}

async fn finish(
    repositories: &PostgresRepositories,
    claim_id: RepositoryClaimId,
    started_at: UtcMillis,
    result: ClaimCheckResult,
) {
    ProjectRoomStore::finish_check(repositories, claim_id, started_at, result)
        .await
        .expect("可写");
}

async fn bind(
    repositories: &PostgresRepositories,
    request: &BindRepository,
) -> BindRepositoryOutcome {
    ProjectRoomStore::bind_room(repositories, request)
        .await
        .expect("写入不报错")
}

async fn binding(
    repositories: &PostgresRepositories,
    catalog_id: RoomCatalogId,
) -> Option<RepositoryBindingRecord> {
    ProjectRoomStore::binding_for_room(repositories, catalog_id)
        .await
        .expect("可读")
}

async fn unbind(
    repositories: &PostgresRepositories,
    catalog_id: RoomCatalogId,
    owner: PrincipalId,
) -> UnbindRepositoryOutcome {
    ProjectRoomStore::unbind_by_owner(repositories, catalog_id, owner)
        .await
        .expect("写入不报错")
}
