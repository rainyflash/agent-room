//! 子房间第 2d-1 步（`specs/redesign/sub-rooms.md` 的“门”和“加密和谁读得到”）：建的时候带门，之后
//! 装门、拆门、换一种门，和“不在名单里显示”。申请、批准、请进来在后面几步。
//!
//! 夹具是 `sub_rooms.rs` 里的那一份。装门开加密、拆门写下那一刻走的是公开设置那两个口子
//! （`PrivateRoomListingMatrix` 和 `PrivateRoomListingStore::mark_room_encrypted`），这里给 Matrix 和
//! 存储的替身各补上一份：落在哪间房照旧记在 Matrix 替身的改动里。

use agent_room_application::ports::{
    MatrixRoomEncryption, PrivateRoomListingMatrix, PrivateRoomListingRecord,
    PrivateRoomListingSaveOutcome, PrivateRoomListingStore, SubRoomPrivacy,
};
use agent_room_domain::room_listing::{RoomListingSettings, RoomShareCode};

use super::{
    membership::{refusal, sub_room},
    sync::room,
    *,
};

const REVISED: [&str; 3] = ["sub_rooms.revise", "matrix.set_name", "matrix.set_topic"];

fn names(list: &agent_room_application::private_rooms::SubRoomList) -> Vec<&str> {
    list.sub_rooms
        .iter()
        .map(|view| view.entry.name.as_str())
        .collect()
}

fn inside(rooms: &Rooms, catalog: u128, person: u128) -> bool {
    rooms
        .store
        .room(catalog)
        .member(principal_id(person))
        .is_some_and(PrivateRoomMember::has_joined)
}

#[tokio::test]
async fn 建的时候就带门_父房间不加密它也加密_藏着的别的成员看不到() {
    let rooms = Rooms::new();
    rooms.store.set_encrypted(PARENT, false);
    rooms
        .service
        .create_sub_room(CreateSubRoom {
            access: RoomJoinMode::Request,
            hidden: true,
            ..Rooms::request(MEMBER, 20, "security")
        })
        .await
        .expect("能发言的成员能建带门的");

    let creation = rooms.matrix.creation().expect("建了 Matrix 房间");
    assert_eq!(
        creation.request().encryption(),
        MatrixRoomEncryption::EndToEnd,
        "有门的一律加密"
    );
    let link = rooms.store.link(20).link;
    assert_eq!(link.access, RoomJoinMode::Request);
    assert!(link.hidden);
    assert!(link.end_to_end_encrypted);
    assert_eq!(link.door_opened_at, None);

    // 没有门的照旧跟父房间。
    rooms.create(MEMBER, 21, "open").await.expect("没有门的");
    assert!(!rooms.store.link(21).link.end_to_end_encrypted);

    let seen_by_viewer = rooms.list(VIEWER, PARENT).await.expect("能读名单");
    assert_eq!(names(&seen_by_viewer), ["open"]);
    let seen_by_admin = rooms.list(ADMIN, PARENT).await.expect("能读名单");
    assert_eq!(
        names(&seen_by_admin),
        ["security", "open"],
        "房间里没有房主和管理员看不见的地方"
    );
}

#[tokio::test]
async fn 没有门的不能藏_同一个请求重发认门_门不一样算撞号() {
    let rooms = Rooms::new();
    let hidden_without_door = CreateSubRoom {
        hidden: true,
        ..Rooms::request(MEMBER, 20, "security")
    };
    assert_eq!(
        refusal(rooms.service.create_sub_room(hidden_without_door).await),
        PrivateRoomFailureKind::InvalidRequest
    );
    assert!(rooms.events().is_empty(), "什么都没动");

    let invite_only = || CreateSubRoom {
        access: RoomJoinMode::InviteOnly,
        ..Rooms::request(MEMBER, 20, "security")
    };
    let created = rooms
        .service
        .create_sub_room(invite_only())
        .await
        .expect("建得出");
    rooms.clear_events();
    let again = rooms
        .service
        .create_sub_room(invite_only())
        .await
        .expect("重发回同一间");
    assert_eq!(again, created);
    assert!(rooms.events().is_empty());

    assert_eq!(
        refusal(rooms.create(MEMBER, 20, "security").await),
        PrivateRoomFailureKind::Conflict,
        "同一个房间号、门不一样，是另一个请求"
    );
}

#[tokio::test]
async fn 装门_不加密的先开加密再存门_里面的人不动_外面的要批准() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed {
        encrypted: false,
        ..Seed::open(20, "release")
    });
    rooms.join(MEMBER, 20).await.expect("没有门的点一下就进");
    rooms.clear_events();
    rooms.matrix.clear_changes();

    rooms
        .change_door(ADMIN, 20, Some(RoomJoinMode::Request), None)
        .await
        .expect("父房间的管理员没进来也能装门");
    assert_eq!(
        rooms.events(),
        [
            "matrix.encrypt",
            "store.mark_encrypted",
            "sub_rooms.revise",
            "matrix.set_name",
            "matrix.set_topic"
        ],
        "先开加密，再存门"
    );
    assert_eq!(rooms.matrix.changes_in(&room(20)), ["encrypt"]);
    let link = rooms.store.link(20).link;
    assert_eq!(link.access, RoomJoinMode::Request);
    assert!(link.end_to_end_encrypted);
    assert_eq!(link.name_key, "release", "没带名字，名字不动");
    assert!(inside(&rooms, 20, MEMBER), "装门不会把已经在里面的人请出去");
    assert_eq!(
        refusal(rooms.join(VIEWER, 20).await),
        sub_room(SubRoomRefusal::ApprovalRequired)
    );

    // 换一种门：已经加密了，不用再开。
    rooms.clear_events();
    rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::InviteOnly), None)
        .await
        .expect("房主能换门");
    assert_eq!(rooms.events(), REVISED);
    assert_eq!(
        refusal(rooms.join(VIEWER, 20).await),
        sub_room(SubRoomRefusal::InviteOnly)
    );
}

#[tokio::test]
async fn 不在名单里显示_有门的才能勾_勾上以后外面的人找不到这一间() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed::open(20, "release"));
    rooms.clear_events();

    assert_eq!(
        refusal(rooms.change_door(OWNER, 20, None, Some(true)).await),
        PrivateRoomFailureKind::InvalidRequest,
        "谁都进得来的房间藏不住"
    );
    assert!(rooms.events().is_empty(), "什么都没动");

    rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::Request), Some(true))
        .await
        .expect("装门的同时勾上");
    assert!(rooms.store.link(20).link.hidden);
    assert_eq!(
        refusal(rooms.join(VIEWER, 20).await),
        PrivateRoomFailureKind::NotFound,
        "和没有这间子房间一样回"
    );
    assert!(names(&rooms.list(VIEWER, PARENT).await.expect("能读名单")).is_empty());

    // 换一种门不动隐藏。
    rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::InviteOnly), None)
        .await
        .expect("换一种门");
    let link = rooms.store.link(20).link;
    assert_eq!(link.access, RoomJoinMode::InviteOnly);
    assert!(link.hidden);

    // 拆门还明说要藏：拒绝，什么都不动。
    rooms.clear_events();
    let open_but_hidden = rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::Open), Some(true))
        .await;
    assert_eq!(
        refusal(open_but_hidden),
        PrivateRoomFailureKind::InvalidRequest
    );
    assert!(rooms.events().is_empty(), "什么都没动");

    // 只去掉隐藏：门还在，名单里看得到、进不去。
    rooms
        .change_door(OWNER, 20, None, Some(false))
        .await
        .expect("去掉隐藏");
    assert_eq!(
        names(&rooms.list(VIEWER, PARENT).await.expect("能读名单")),
        ["release"]
    );
    assert_eq!(
        refusal(rooms.join(VIEWER, 20).await),
        sub_room(SubRoomRefusal::InviteOnly)
    );
}

#[tokio::test]
async fn 拆门_先写下这一刻再开门_隐藏跟着去掉_加密还在_再装再拆不再写() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed {
        access: RoomJoinMode::Request,
        hidden: true,
        ..Seed::open(20, "security")
    });
    rooms.clear_events();

    rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::Open), None)
        .await
        .expect("房主能拆门");
    assert_eq!(
        rooms.events(),
        [
            "matrix.listed",
            "sub_rooms.revise",
            "matrix.set_name",
            "matrix.set_topic"
        ],
        "这以后进来的人不补发钥匙：这一刻要写在门开之前"
    );
    assert_eq!(rooms.matrix.changes_in(&room(20)), ["listed 1000"]);
    let opened_at = Some(UtcMillis::new(1_000).expect("时间有效"));
    let link = rooms.store.link(20).link;
    assert_eq!(link.access, RoomJoinMode::Open);
    assert!(!link.hidden, "没有门的藏不住，隐藏跟着去掉");
    assert_eq!(link.door_opened_at, opened_at);
    assert!(link.end_to_end_encrypted, "加密开了关不掉");
    rooms.join(VIEWER, 20).await.expect("拆了门谁都能进");

    // 再装上、再拆：那一刻不往后挪。挪了的话，第一次拆门以后自己进来的人就成了“拆门以前就在”。
    rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::InviteOnly), None)
        .await
        .expect("再装上");
    rooms.clear_events();
    rooms.matrix.clear_changes();
    rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::Open), None)
        .await
        .expect("再拆");
    assert_eq!(rooms.events(), REVISED);
    assert!(rooms.matrix.changes_in(&room(20)).is_empty());
    assert_eq!(rooms.store.link(20).link.door_opened_at, opened_at);
}

#[tokio::test]
async fn 聊天服务器上没做成就不换门_好了以后再点一次接得上() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed {
        encrypted: false,
        ..Seed::open(20, "release")
    });
    rooms.store.seed(&Seed {
        access: RoomJoinMode::Request,
        ..Seed::open(21, "security")
    });
    for catalog in [20, 21] {
        rooms
            .matrix
            .fail_in(&room(catalog), MatrixFailureKind::DependencyUnavailable);
    }
    rooms.clear_events();

    let installing = rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::Request), None)
        .await;
    assert_eq!(
        refusal(installing),
        PrivateRoomFailureKind::DependencyUnavailable
    );
    let removing = rooms
        .change_door(OWNER, 21, Some(RoomJoinMode::Open), None)
        .await;
    assert_eq!(
        refusal(removing),
        PrivateRoomFailureKind::DependencyUnavailable
    );
    assert!(rooms.events().is_empty(), "什么都没存");
    let unencrypted = rooms.store.link(20).link;
    assert_eq!(unencrypted.access, RoomJoinMode::Open);
    assert!(!unencrypted.end_to_end_encrypted);
    let behind_door = rooms.store.link(21).link;
    assert_eq!(behind_door.access, RoomJoinMode::Request);
    assert_eq!(behind_door.door_opened_at, None);

    for catalog in [20, 21] {
        rooms.matrix.recover(&room(catalog));
    }
    rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::Request), None)
        .await
        .expect("再点一次装上了");
    rooms
        .change_door(OWNER, 21, Some(RoomJoinMode::Open), None)
        .await
        .expect("再点一次拆掉了");
    assert!(rooms.store.link(20).link.end_to_end_encrypted);
    assert!(rooms.store.link(21).link.door_opened_at.is_some());
}

#[tokio::test]
async fn 装门的同时改名撞了名字_门没装上_加密已经开了不要紧() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed {
        encrypted: false,
        ..Seed::open(20, "release")
    });
    rooms.store.seed(&Seed::open(21, "website"));
    rooms.clear_events();

    let taken = rooms
        .service
        .revise_sub_room(ReviseSubRoom {
            actor: person(OWNER),
            parent_catalog_id: catalog_id(PARENT),
            catalog_id: catalog_id(20),
            name: Some("website".to_owned()),
            description: None,
            access: Some(RoomJoinMode::Request),
            hidden: None,
            category: None,
        })
        .await;
    assert_eq!(refusal(taken), sub_room(SubRoomRefusal::NameTaken));
    assert_eq!(
        rooms.events(),
        ["matrix.encrypt", "store.mark_encrypted", "sub_rooms.revise"]
    );
    let link = rooms.store.link(20).link;
    assert_eq!(link.access, RoomJoinMode::Open, "门没装上");
    assert!(
        link.end_to_end_encrypted,
        "没有门、加密着：和拆过门的子房间一样，目录和聊天服务器对得上"
    );
    rooms
        .join(VIEWER, 20)
        .await
        .expect("没有门，照旧点一下就进");

    // 换个不重名的再装：加密已经开了，不再开一遍。
    rooms.clear_events();
    rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::Request), None)
        .await
        .expect("装上了");
    assert_eq!(rooms.events(), REVISED);
}

#[tokio::test]
async fn 存的当口别人换了门_这一次不写_只改名的不把读到的旧门写回去() {
    let rooms = Rooms::new();
    rooms.store.seed(&Seed {
        access: RoomJoinMode::Request,
        ..Seed::open(20, "security")
    });
    rooms.clear_events();

    // 他读到的是“要批准”，照它算的是拆门；存之前别人把门换成了“只能请进来”。
    rooms
        .store
        .move_door_before_revision(20, RoomJoinMode::InviteOnly);
    let raced = rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::Open), None)
        .await;
    assert_eq!(
        refusal(raced),
        PrivateRoomFailureKind::Conflict,
        "请他看过新的再换"
    );
    assert_eq!(
        rooms.events(),
        ["matrix.listed", "sub_rooms.revise"],
        "那一刻已经写下了，门却没开：不要紧，真拆的时候再写一遍"
    );
    let link = rooms.store.link(20).link;
    assert_eq!(link.access, RoomJoinMode::InviteOnly, "别人换的门还在");
    assert_eq!(link.door_opened_at, None);

    // 只改名的请求不带门：存的当口门被换了也照改，读到的旧门不写回去。
    rooms
        .store
        .move_door_before_revision(20, RoomJoinMode::Request);
    rooms
        .revise(OWNER, 20, Some("vault"), None)
        .await
        .expect("改名不看门");
    let link = rooms.store.link(20).link;
    assert_eq!(link.name_key, "vault");
    assert_eq!(link.access, RoomJoinMode::Request, "没把读到的旧门写回去");

    rooms.clear_events();
    rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::Open), None)
        .await
        .expect("看过新的再拆");
    assert_eq!(
        rooms.events(),
        [
            "matrix.listed",
            "sub_rooms.revise",
            "matrix.set_name",
            "matrix.set_topic"
        ]
    );
    assert!(rooms.store.link(20).link.door_opened_at.is_some());
}

#[tokio::test]
async fn 管得了这一间的人才能换门_归档了的换不了() {
    let rooms = Rooms::new();
    rooms.create(MEMBER, 20, "release").await.expect("成员建的");
    rooms.join(VIEWER, 20).await.expect("只能看的也能进");
    rooms.clear_events();

    let by_viewer = rooms
        .change_door(VIEWER, 20, Some(RoomJoinMode::Request), None)
        .await;
    assert_eq!(refusal(by_viewer), sub_room(SubRoomRefusal::ManagersOnly));
    assert!(rooms.events().is_empty(), "什么都没动");

    rooms
        .change_door(MEMBER, 20, Some(RoomJoinMode::Request), None)
        .await
        .expect("建的人管这一间");
    assert_eq!(rooms.store.link(20).link.access, RoomJoinMode::Request);
    assert!(inside(&rooms, 20, VIEWER), "已经在里面的人不动");

    rooms.store.archive(20);
    rooms.clear_events();
    let archived = rooms
        .change_door(OWNER, 20, Some(RoomJoinMode::Open), None)
        .await;
    assert_eq!(refusal(archived), sub_room(SubRoomRefusal::Archived));
    assert!(rooms.events().is_empty(), "归档了的要先恢复");
}

// ---- 替身

impl PrivateRoomListingMatrix for TestMatrix {
    fn mark_listed<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        first_listed_at: UtcMillis,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.change(
                room_id,
                MatrixOperation::SendStateEvent,
                format!("listed {}", first_listed_at.value()),
            )?;
            self.record("matrix.listed");
            Ok(())
        })
    }

    fn enable_encryption<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.change(
                room_id,
                MatrixOperation::SendStateEvent,
                "encrypt".to_owned(),
            )?;
            self.record("matrix.encrypt");
            Ok(())
        })
    }
}

/// 子房间不能公开，父房间公开着只摆出“正公开着”（`set_listed`）；用得上的是“目录上标成加密”和
/// 子房间跟着父房间写下那一刻（2d-5）。
impl PrivateRoomListingStore for MemoryRooms {
    fn listing(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomListingRecord>>> {
        let listed = self
            .listed
            .lock()
            .expect("公开锁正常")
            .contains(&catalog_id);
        let record = listed.then(|| PrivateRoomListingRecord {
            catalog_id,
            settings: RoomListingSettings::new(
                true,
                RoomJoinMode::Open,
                RoomListingSettings::DEFAULT_MEMBER_PERMISSIONS,
                true,
            )
            .expect("设置有效"),
            share_code: RoomShareCode::parse("Par001").expect("分享码有效"),
            first_listed_at: UtcMillis::new(500).expect("时间有效"),
            updated_by: principal_id(OWNER),
            updated_at: UtcMillis::new(500).expect("时间有效"),
            version: AggregateVersion::INITIAL,
        });
        Box::pin(async move { Ok(record) })
    }

    fn sub_room_privacy(
        &self,
        parent_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<SubRoomPrivacy>>> {
        let rooms = self.rooms.lock().expect("房间锁正常");
        let sub_rooms = self
            .links
            .lock()
            .expect("关系锁正常")
            .iter()
            .filter(|stored| stored.link.parent_catalog_id == parent_catalog_id)
            .map(|stored| SubRoomPrivacy {
                catalog_id: stored.link.catalog_id,
                matrix_room: MatrixRoomId::new(
                    rooms[&stored.link.catalog_id]
                        .instance()
                        .matrix_room_id()
                        .as_str()
                        .to_owned(),
                )
                .expect("房间有效"),
                has_door: stored.link.access != RoomJoinMode::Open,
                listing_marked: stored.link.door_opened_at.is_some(),
                end_to_end_encrypted: stored.link.end_to_end_encrypted,
            })
            .collect();
        Box::pin(async move { Ok(sub_rooms) })
    }

    fn mark_sub_room_listed(
        &self,
        catalog_id: RoomCatalogId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        self.record("store.mark_listed");
        for stored in self.links.lock().expect("关系锁正常").iter_mut() {
            if stored.link.catalog_id == catalog_id && stored.link.door_opened_at.is_none() {
                stored.link.door_opened_at = Some(at);
            }
        }
        Box::pin(async { Ok(()) })
    }

    fn listed_rooms_owned_by(
        &self,
        _owner: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomCatalogId>>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn save_listing<'a>(
        &'a self,
        _record: &'a PrivateRoomListingRecord,
        _expected: Option<AggregateVersion>,
        _owner: PrincipalId,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        Box::pin(async { unreachable!("子房间的用例不改公开设置") })
    }

    fn create_listed_room<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _listing: &'a PrivateRoomListingRecord,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        Box::pin(async { unreachable!("子房间的用例不建公开房间") })
    }

    fn save_transferred_room<'a>(
        &'a self,
        room: &'a PrivateRoom,
        expected: AggregateVersion,
        changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        Box::pin(async move {
            PrivateRoomStore::save(self, room, expected, changed_at).await?;
            Ok(PrivateRoomListingSaveOutcome::Saved)
        })
    }

    fn mark_room_encrypted(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        self.record("store.mark_encrypted");
        self.unencrypted
            .lock()
            .expect("加密锁正常")
            .remove(&catalog_id);
        for stored in self.links.lock().expect("关系锁正常").iter_mut() {
            if stored.link.catalog_id == catalog_id {
                stored.link.end_to_end_encrypted = true;
            }
        }
        Box::pin(async { Ok(()) })
    }
}
