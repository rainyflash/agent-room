//! 子房间管理员的存储（`specs/redesign/room-roles.md` 的 R5，子房间第 2d-4 步）：身上的“管理”、管理员
//! 名单和身份变动在同一个事务里改；存的当口他已经不在父房间里了就什么都不写。读父房间的身份变动时
//! 连它下面子房间里有人动手的一起读，写着是哪一间；跟着父房间重算带出来的不重复列。

use agent_room_application::ports::{
    RoomRoleChange, RoomRoleChangeLog, RoomRoleChangeRecord, SubRoomManagerChange,
};
use agent_room_domain::{
    private_rooms::{PrivateRoomCapability, PrivateRoomPermissions},
    room_role_changes::standing_changes,
    sub_rooms::sub_room_permissions,
};

use super::{
    follow::id,
    membership::{change, enter},
    *,
};

/// 一间房，下面一间叫 Release 的子房间；四个人都在两间里。
struct Fixture {
    database: TestDatabase,
    repositories: PostgresRepositories,
    parent: RoomCatalogId,
    release: RoomCatalogId,
    owner: PrincipalId,
    granter: PrincipalId,
    member: PrincipalId,
    other: PrincipalId,
}

impl Fixture {
    async fn new(prefix: &str) -> Self {
        let database = TestDatabase::connect().await;
        let repositories = PostgresRepositories::new(database.runtime.clone());
        let pool = &database.runtime;
        let owner = seed_principal(pool, &format!("{prefix}-owner")).await;
        let granter = seed_principal(pool, &format!("{prefix}-granter")).await;
        let member = seed_principal(pool, &format!("{prefix}-member")).await;
        let other = seed_principal(pool, &format!("{prefix}-other")).await;
        let parent = create_room(&repositories, owner).await;
        for person in [granter, member, other] {
            admit(&repositories, parent, owner, person).await;
        }
        let release = draft(&repositories, parent, owner, "Release", time(10)).await;
        assert_eq!(create(&repositories, &release).await, None);
        let release = id(&release);
        for person in [granter, member, other] {
            assert!(enter(&repositories, parent, release, person, time(20)).await);
        }
        Self {
            database,
            repositories,
            parent,
            release,
            owner,
            granter,
            member,
            other,
        }
    }

    /// 照用例那样设、撤 `member`：权限照他在父房间里的算，以房主的名义改，身份变动记在 `granter`
    /// 名下。权限要和用例算的一样：不一样的话，父房间一变、子房间照着重算时会多记一笔。
    async fn set_member_manager(&self, grant: bool, at: UtcMillis) -> bool {
        let in_parent = load(&self.repositories, self.parent)
            .await
            .room()
            .member(self.member)
            .expect("在父房间里")
            .permissions();
        let permissions = sub_room_permissions(in_parent, grant).expect("在父房间里看得到");
        let before = load(&self.repositories, self.release).await;
        let mut room = before.room().clone();
        room.update_permissions(room.owner_principal_id(), self.member, permissions)
            .expect("房主能改权限");
        let role_changes: Vec<RoomRoleChange> = standing_changes(before.room(), &room)
            .into_iter()
            .map(|change| RoomRoleChange {
                actor: Some(self.granter),
                change,
            })
            .collect();
        self.store(&room, before.room().version(), grant, at, &role_changes)
            .await
    }

    async fn store(
        &self,
        room: &PrivateRoom,
        expected_version: AggregateVersion,
        grant: bool,
        at: UtcMillis,
        role_changes: &[RoomRoleChange],
    ) -> bool {
        SubRoomStore::change_sub_room_manager(
            &self.repositories,
            &SubRoomManagerChange {
                room,
                expected_version,
                parent_catalog_id: self.parent,
                principal: self.member,
                grant,
                changed_by: self.granter,
                at,
                role_changes,
            },
        )
        .await
        .expect("写入不报错")
    }

    /// 管理员名单：谁、谁设的。
    async fn managers(&self) -> Vec<(PrincipalId, PrincipalId)> {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            r"SELECT principal_id, granted_by_principal_id
                FROM agent_room.private_room_sub_room_manager
               WHERE catalog_entry_id = $1
               ORDER BY granted_at, principal_id",
        )
        .bind(self.release.as_uuid())
        .fetch_all(&self.database.runtime)
        .await
        .expect("可读");
        rows.into_iter()
            .map(|(principal, granted_by)| {
                (
                    PrincipalId::from_uuid(principal),
                    PrincipalId::from_uuid(granted_by),
                )
            })
            .collect()
    }

    /// `member` 在子房间里：进没进来、有没有“管理”。
    async fn member_standing(&self) -> Option<(bool, bool)> {
        load(&self.repositories, self.release)
            .await
            .room()
            .member(self.member)
            .map(|stored| (stored.has_joined(), stored.permissions().manage()))
    }

    async fn log(&self, catalog: RoomCatalogId) -> Vec<RoomRoleChangeRecord> {
        RoomRoleChangeLog::recent_role_changes(&self.repositories, catalog, 100)
            .await
            .expect("可读")
    }
}

fn holding(capabilities: &[PrivateRoomCapability]) -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities(capabilities.iter().copied()).expect("权限有效")
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 设撤子房间管理员_管理和名单一起改_身份变动记在动手的人名下() {
    let rooms = Fixture::new("sub-room-manager-grant").await;

    assert!(rooms.set_member_manager(true, time(30)).await);
    assert_eq!(rooms.member_standing().await, Some((true, true)));
    assert_eq!(rooms.managers().await, [(rooms.member, rooms.granter)]);
    let own = rooms.log(rooms.release).await;
    assert_eq!(own.len(), 1);
    assert_eq!(
        (own[0].actor, own[0].target),
        (Some(rooms.granter), rooms.member)
    );
    assert_eq!(own[0].sub_room, None, "子房间自己读的不写是哪一间");

    // 撤：名单上没了，“管理”也没了，人还在。
    assert!(rooms.set_member_manager(false, time(50)).await);
    assert!(rooms.managers().await.is_empty());
    assert_eq!(rooms.member_standing().await, Some((true, false)));
    assert_eq!(rooms.log(rooms.release).await.len(), 2);

    rooms.database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 父房间读得到子房间里有人动手的_写着哪一间_重算带出来的不重复列() {
    let rooms = Fixture::new("sub-room-manager-log").await;
    assert!(rooms.set_member_manager(true, time(30)).await);

    // 父房间里设了一位管理员：子房间里跟着重算，记一笔“谁动的手”为空的。
    let (owner, other) = (rooms.owner, rooms.other);
    change(&rooms.repositories, rooms.parent, time(40), |room| {
        let admin = holding(&[
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Manage,
        ]);
        room.update_permissions(owner, other, admin)
            .expect("房主能设管理员");
    })
    .await;
    let derived = rooms.log(rooms.release).await;
    assert_eq!(derived.len(), 2, "子房间里照着重算的记了一笔");
    assert_eq!((derived[0].actor, derived[0].target), (None, other));

    let above = rooms.log(rooms.parent).await;
    let in_release: Vec<_> = above
        .iter()
        .filter(|record| record.sub_room.is_some())
        .collect();
    assert_eq!(in_release.len(), 1, "只列有人动手的那一笔");
    assert_eq!(in_release[0].target, rooms.member);
    let named = in_release[0].sub_room.as_ref().expect("写着哪一间");
    assert_eq!(
        (named.catalog_id, named.name.as_str()),
        (rooms.release, "Release")
    );
    assert!(
        above
            .iter()
            .any(|record| record.sub_room.is_none() && record.target == other),
        "父房间自己的照旧在"
    );
    assert!(
        above.windows(2).all(|pair| pair[0].id > pair[1].id),
        "新的在前"
    );

    rooms.database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 存的当口他已经被移出了父房间_什么都不写() {
    let rooms = Fixture::new("sub-room-manager-gone").await;
    let (owner, member) = (rooms.owner, rooms.member);
    change(&rooms.repositories, rooms.parent, time(60), |room| {
        room.remove_member(owner, member).expect("房主能移出");
    })
    .await;

    // 移出时子房间里他也跟着出去了。拿此刻读到的子房间去存（随便改一处），存储锁着看一眼父房间
    // 就不写。
    let stale = load(&rooms.repositories, rooms.release).await;
    let mut room = stale.room().clone();
    let manages = holding(&[
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
        PrivateRoomCapability::Manage,
    ]);
    room.update_permissions(owner, rooms.granter, manages)
        .expect("房主能改权限");
    assert!(
        !rooms
            .store(&room, stale.room().version(), true, time(70), &[])
            .await,
        "他不在父房间里了"
    );
    assert_eq!(
        load(&rooms.repositories, rooms.release)
            .await
            .room()
            .version(),
        stale.room().version()
    );
    assert!(rooms.managers().await.is_empty());

    rooms.database.close().await;
}
