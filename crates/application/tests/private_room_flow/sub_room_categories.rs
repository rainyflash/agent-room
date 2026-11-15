//! 子房间第 2e-1 步：分类。房间的房主和管理员建、改名、排先后、删；一间房最多 20 个，名字不重名
//! （不分大小写）；名单里带着分类。夹具是 `sub_rooms.rs` 里的那一份，存储的规则照真实存储的写，
//! 真实存储自己的测试在 `postgres-adapter` 的 `tests/private_rooms/sub_room_categories.rs`。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        NewSubRoomCategory, PortFuture, SubRoomCategory, SubRoomCategoryOutcome,
        SubRoomCategoryRemoval, SubRoomCategoryRevision, SubRoomCategoryStore,
    },
    private_rooms::{
        CreateSubRoom, CreateSubRoomCategory, DeleteSubRoomCategory, ReviseSubRoom,
        ReviseSubRoomCategory,
    },
};
use agent_room_domain::{ids::SubRoomCategoryId, sub_room_categories::MAXIMUM_SUB_ROOM_CATEGORIES};

use super::{
    membership::{refusal, sub_room},
    *,
};

/// 存着的一个分类。
#[derive(Debug, Clone)]
pub(super) struct StoredCategory {
    parent: RoomCatalogId,
    name_key: String,
    category: SubRoomCategory,
}

fn category_id(value: u128) -> SubRoomCategoryId {
    SubRoomCategoryId::from_uuid(uuid(value))
}

impl MemoryRooms {
    /// 这个分类在不在这间房里。
    pub(super) fn has_category(&self, parent: RoomCatalogId, category: SubRoomCategoryId) -> bool {
        self.categories
            .lock()
            .expect("分类锁正常")
            .iter()
            .any(|stored| stored.parent == parent && stored.category.id == category)
    }

    /// 父房间还在、没归档、自己不是子房间。
    fn category_parent_usable(&self, parent: RoomCatalogId) -> bool {
        self.rooms
            .lock()
            .expect("房间锁正常")
            .get(&parent)
            .is_some_and(|snapshot| {
                snapshot.parent_catalog_id().is_none()
                    && snapshot.room().status() == PrivateRoomLifecycleStatus::Active
            })
    }

    fn categories_of(stored: &[StoredCategory], parent: RoomCatalogId) -> Vec<SubRoomCategory> {
        let mut categories: Vec<SubRoomCategory> = stored
            .iter()
            .filter(|stored| stored.parent == parent)
            .map(|stored| stored.category.clone())
            .collect();
        categories.sort_by_key(|category| category.position);
        categories
    }

    /// 这间房的分类照现在的先后重新从 0 起编号，`moved` 先挪到 `to`。
    fn renumber(
        stored: &mut [StoredCategory],
        parent: RoomCatalogId,
        moved: Option<(SubRoomCategoryId, u8)>,
    ) {
        let mut order: Vec<SubRoomCategoryId> = Self::categories_of(stored, parent)
            .into_iter()
            .map(|category| category.id)
            .collect();
        if let Some((id, to)) = moved {
            order.retain(|existing| *existing != id);
            let at = usize::from(to).min(order.len());
            order.insert(at, id);
        }
        for (position, id) in order.into_iter().enumerate() {
            let entry = stored
                .iter_mut()
                .find(|stored| stored.category.id == id)
                .expect("分类存在");
            entry.category.position = u8::try_from(position).expect("分类不超过上限");
        }
    }
}

impl SubRoomCategoryStore for MemoryRooms {
    fn sub_room_categories(
        &self,
        parent: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<SubRoomCategory>>> {
        Box::pin(async move {
            let stored = self.categories.lock().expect("分类锁正常");
            Ok(Self::categories_of(&stored, parent))
        })
    }

    fn create_sub_room_category<'a>(
        &'a self,
        category: &'a NewSubRoomCategory,
    ) -> PortFuture<'a, RepositoryResult<SubRoomCategoryOutcome>> {
        Box::pin(async move {
            if !self.category_parent_usable(category.parent_catalog_id) {
                return Ok(SubRoomCategoryOutcome::ParentUnavailable);
            }
            let parent = category.parent_catalog_id;
            let mut stored = self.categories.lock().expect("分类锁正常");
            if let Some(existing) = stored
                .iter()
                .find(|stored| stored.category.id == category.id)
            {
                return Ok(
                    if existing.parent == parent && existing.name_key == category.name.name_key {
                        SubRoomCategoryOutcome::Saved(Self::categories_of(&stored, parent))
                    } else {
                        SubRoomCategoryOutcome::IdTaken
                    },
                );
            }
            let siblings = Self::categories_of(&stored, parent);
            if stored
                .iter()
                .any(|stored| stored.parent == parent && stored.name_key == category.name.name_key)
            {
                return Ok(SubRoomCategoryOutcome::NameTaken);
            }
            if siblings.len() >= usize::from(MAXIMUM_SUB_ROOM_CATEGORIES) {
                return Ok(SubRoomCategoryOutcome::LimitReached);
            }
            stored.push(StoredCategory {
                parent,
                name_key: category.name.name_key.clone(),
                category: SubRoomCategory {
                    id: category.id,
                    name: category.name.name.clone(),
                    position: u8::try_from(siblings.len()).expect("分类不超过上限"),
                },
            });
            self.record("sub_rooms.category");
            Ok(SubRoomCategoryOutcome::Saved(Self::categories_of(
                &stored, parent,
            )))
        })
    }

    fn revise_sub_room_category<'a>(
        &'a self,
        revision: &'a SubRoomCategoryRevision,
    ) -> PortFuture<'a, RepositoryResult<SubRoomCategoryOutcome>> {
        Box::pin(async move {
            if !self.category_parent_usable(revision.parent_catalog_id) {
                return Ok(SubRoomCategoryOutcome::ParentUnavailable);
            }
            let parent = revision.parent_catalog_id;
            let mut stored = self.categories.lock().expect("分类锁正常");
            if !stored
                .iter()
                .any(|stored| stored.parent == parent && stored.category.id == revision.id)
            {
                return Ok(SubRoomCategoryOutcome::Missing);
            }
            if let Some(name) = &revision.name {
                if stored.iter().any(|stored| {
                    stored.parent == parent
                        && stored.category.id != revision.id
                        && stored.name_key == name.name_key
                }) {
                    return Ok(SubRoomCategoryOutcome::NameTaken);
                }
                let entry = stored
                    .iter_mut()
                    .find(|stored| stored.category.id == revision.id)
                    .expect("分类存在");
                entry.name_key.clone_from(&name.name_key);
                entry.category.name.clone_from(&name.name);
            }
            Self::renumber(
                &mut stored,
                parent,
                revision.position.map(|to| (revision.id, to)),
            );
            self.record("sub_rooms.category");
            Ok(SubRoomCategoryOutcome::Saved(Self::categories_of(
                &stored, parent,
            )))
        })
    }

    fn delete_sub_room_category<'a>(
        &'a self,
        removal: &'a SubRoomCategoryRemoval,
    ) -> PortFuture<'a, RepositoryResult<SubRoomCategoryOutcome>> {
        Box::pin(async move {
            if !self.category_parent_usable(removal.parent_catalog_id) {
                return Ok(SubRoomCategoryOutcome::ParentUnavailable);
            }
            let parent = removal.parent_catalog_id;
            let mut stored = self.categories.lock().expect("分类锁正常");
            let before = stored.len();
            stored.retain(|stored| !(stored.parent == parent && stored.category.id == removal.id));
            let removed = stored.len() != before;
            if removed {
                Self::renumber(&mut stored, parent, None);
                self.record("sub_rooms.category");
            }
            let left = Self::categories_of(&stored, parent);
            drop(stored);
            // 里面的子房间回到“未分类”，和真实存储的外键置空一样。
            if removed {
                for link in self.links.lock().expect("关系锁正常").iter_mut() {
                    if link.category == Some(removal.id) {
                        link.category = None;
                    }
                }
            }
            Ok(SubRoomCategoryOutcome::Saved(left))
        })
    }
}

impl Rooms {
    async fn create_category(
        &self,
        actor: u128,
        category: u128,
        name: &str,
    ) -> Result<Vec<SubRoomCategory>, PrivateRoomFailure> {
        self.service
            .create_sub_room_category(CreateSubRoomCategory {
                actor: person(actor),
                parent_catalog_id: catalog_id(PARENT),
                category_id: category_id(category),
                name: name.to_owned(),
            })
            .await
    }

    async fn revise_category(
        &self,
        actor: u128,
        category: u128,
        name: Option<&str>,
        position: Option<u8>,
    ) -> Result<Vec<SubRoomCategory>, PrivateRoomFailure> {
        self.service
            .revise_sub_room_category(ReviseSubRoomCategory {
                actor: person(actor),
                parent_catalog_id: catalog_id(PARENT),
                category_id: category_id(category),
                name: name.map(str::to_owned),
                position,
            })
            .await
    }

    async fn delete_category(
        &self,
        actor: u128,
        category: u128,
    ) -> Result<Vec<SubRoomCategory>, PrivateRoomFailure> {
        self.service
            .delete_sub_room_category(DeleteSubRoomCategory {
                actor: person(actor),
                parent_catalog_id: catalog_id(PARENT),
                category_id: category_id(category),
            })
            .await
    }
}

fn names(categories: &[SubRoomCategory]) -> Vec<(&str, u8)> {
    categories
        .iter()
        .map(|category| (category.name.as_str(), category.position))
        .collect()
}

#[tokio::test]
async fn 房主和管理员建分类_排在最后_两头的空白去掉_重发同一个请求不多建() {
    let rooms = Rooms::new();
    rooms
        .create_category(OWNER, 60, "设计")
        .await
        .expect("房主能建");
    let created = rooms
        .create_category(ADMIN, 61, "  Release  ")
        .await
        .expect("管理员能建");
    assert_eq!(names(&created), [("设计", 0), ("Release", 1)]);
    assert_eq!(
        rooms.events(),
        ["sub_rooms.category", "sub_rooms.category"],
        "只存分类，不碰聊天服务器"
    );

    rooms.clear_events();
    let again = rooms
        .create_category(ADMIN, 61, "release")
        .await
        .expect("同一个请求重发");
    assert_eq!(again, created);
    assert!(rooms.events().is_empty(), "重发不多建");

    assert_eq!(
        refusal(rooms.create_category(ADMIN, 61, "测试").await),
        PrivateRoomFailureKind::Conflict,
        "同一个号换了名字是另一个请求撞了号"
    );
}

#[tokio::test]
async fn 只有房主和管理员能管分类_不在房间里的看不到_子房间里做不了() {
    let rooms = Rooms::new();
    rooms
        .create_category(OWNER, 60, "设计")
        .await
        .expect("房主建的");
    for person in [MEMBER, VIEWER] {
        assert_eq!(
            refusal(rooms.create_category(person, 61, "Release").await),
            sub_room(SubRoomRefusal::CategoryAdminsOnly),
        );
        assert_eq!(
            refusal(
                rooms
                    .revise_category(person, 60, Some("Design"), None)
                    .await
            ),
            sub_room(SubRoomRefusal::CategoryAdminsOnly),
        );
        assert_eq!(
            refusal(rooms.delete_category(person, 60).await),
            sub_room(SubRoomRefusal::CategoryAdminsOnly),
        );
    }
    assert_eq!(
        refusal(rooms.create_category(OUTSIDER, 61, "Release").await),
        PrivateRoomFailureKind::NotFound,
        "不在房间里的人看到的和不存在的房间一样"
    );

    rooms
        .create(OWNER, 20, "release")
        .await
        .expect("建了子房间");
    let in_sub_room = rooms
        .service
        .create_sub_room_category(CreateSubRoomCategory {
            actor: person(OWNER),
            parent_catalog_id: catalog_id(20),
            category_id: category_id(61),
            name: "Release".to_owned(),
        })
        .await;
    assert_eq!(
        refusal(in_sub_room),
        sub_room(SubRoomRefusal::ParentOnly),
        "子房间下面没有分类"
    );
}

#[tokio::test]
async fn 名字不重名_不分大小写_一间房最多二十个_名字不合规矩的不收() {
    let rooms = Rooms::new();
    rooms
        .create_category(OWNER, 60, "Design")
        .await
        .expect("建了");
    assert_eq!(
        refusal(rooms.create_category(OWNER, 61, "design").await),
        sub_room(SubRoomRefusal::CategoryNameTaken),
    );
    for invalid in ["", "   ", "a\nb", &"字".repeat(65)] {
        assert_eq!(
            refusal(rooms.create_category(OWNER, 61, invalid).await),
            PrivateRoomFailureKind::InvalidRequest,
            "{invalid:?}"
        );
    }
    for number in 1..u128::from(MAXIMUM_SUB_ROOM_CATEGORIES) {
        rooms
            .create_category(OWNER, 60 + number, &format!("分类 {number}"))
            .await
            .expect("二十个以内都能建");
    }
    assert_eq!(
        refusal(rooms.create_category(OWNER, 99, "第二十一个").await),
        sub_room(SubRoomRefusal::CategoryLimitReached),
    );
    rooms.delete_category(OWNER, 61).await.expect("删掉一个");
    rooms
        .create_category(OWNER, 99, "第二十一个")
        .await
        .expect("删掉一个就又能建");
}

#[tokio::test]
async fn 改名_挪位置_别的跟着让_超过最后的放到最后() {
    let rooms = Rooms::new();
    for (number, name) in [(60, "设计"), (61, "Release"), (62, "测试")] {
        rooms
            .create_category(OWNER, number, name)
            .await
            .expect("建了");
    }
    let moved = rooms
        .revise_category(ADMIN, 62, None, Some(0))
        .await
        .expect("管理员能挪");
    assert_eq!(names(&moved), [("测试", 0), ("设计", 1), ("Release", 2)]);
    let renamed = rooms
        .revise_category(OWNER, 60, Some(" Design "), Some(200))
        .await
        .expect("改名、挪到最后");
    assert_eq!(
        names(&renamed),
        [("测试", 0), ("Release", 1), ("Design", 2)]
    );
    let same = rooms
        .revise_category(OWNER, 60, Some("design"), None)
        .await
        .expect("改成只差大小写的自己不算重名");
    assert_eq!(names(&same), [("测试", 0), ("Release", 1), ("design", 2)]);

    assert_eq!(
        refusal(
            rooms
                .revise_category(OWNER, 60, Some("RELEASE"), None)
                .await
        ),
        sub_room(SubRoomRefusal::CategoryNameTaken),
    );
    assert_eq!(
        refusal(rooms.revise_category(OWNER, 60, None, None).await),
        PrivateRoomFailureKind::InvalidRequest,
        "一项都没带"
    );
    assert_eq!(
        refusal(rooms.revise_category(OWNER, 70, Some("别的"), None).await),
        sub_room(SubRoomRefusal::CategoryMissing),
    );
}

#[tokio::test]
async fn 删掉以后后面的往前挪_再删一次也算成功_名单里带着分类() {
    let rooms = Rooms::new();
    for (number, name) in [(60, "设计"), (61, "Release"), (62, "测试")] {
        rooms
            .create_category(OWNER, number, name)
            .await
            .expect("建了");
    }
    let left = rooms.delete_category(ADMIN, 60).await.expect("管理员能删");
    assert_eq!(names(&left), [("Release", 0), ("测试", 1)]);
    rooms.clear_events();
    let again = rooms.delete_category(ADMIN, 60).await.expect("再删一次");
    assert_eq!(again, left);
    assert!(rooms.events().is_empty(), "删过的不再写");

    let list = rooms
        .list(VIEWER, PARENT)
        .await
        .expect("房间里的人都读得到");
    assert_eq!(names(&list.categories), [("Release", 0), ("测试", 1)]);
}

#[tokio::test]
async fn 父房间归档了就管不了分类() {
    let rooms = Rooms::new();
    rooms
        .create_category(OWNER, 60, "设计")
        .await
        .expect("建了");
    rooms.store.archive(PARENT);
    assert_eq!(
        refusal(rooms.create_category(OWNER, 61, "Release").await),
        PrivateRoomFailureKind::Conflict,
    );
    assert_eq!(
        refusal(rooms.revise_category(OWNER, 60, Some("Design"), None).await),
        PrivateRoomFailureKind::Conflict,
    );
}

impl Rooms {
    /// 只挪分类：`Some(None)` 是挪回“未分类”。
    async fn move_to_category(
        &self,
        person: u128,
        catalog: u128,
        category: Option<u128>,
    ) -> Result<PrivateRoomSnapshot, PrivateRoomFailure> {
        self.service
            .revise_sub_room(ReviseSubRoom {
                actor: self::person(person),
                parent_catalog_id: catalog_id(PARENT),
                catalog_id: catalog_id(catalog),
                name: None,
                description: None,
                access: None,
                hidden: None,
                category: Some(category.map(category_id)),
            })
            .await
    }

    async fn category_of(&self, viewer: u128, catalog: u128) -> Option<SubRoomCategoryId> {
        self.list(viewer, PARENT)
            .await
            .expect("房间里的人读得到")
            .sub_rooms
            .into_iter()
            .find(|view| view.entry.catalog_id == catalog_id(catalog))
            .expect("名单里有这一间")
            .entry
            .category_id
    }
}

#[tokio::test]
async fn 建子房间时放进分类_名单里带着_选的分类没了就建不了_删掉分类回到未分类() {
    let rooms = Rooms::new();
    rooms
        .create_category(ADMIN, 60, "Release")
        .await
        .expect("建了分类");
    rooms
        .service
        .create_sub_room(CreateSubRoom {
            category_id: Some(category_id(60)),
            ..Rooms::request(MEMBER, 20, "ship")
        })
        .await
        .expect("成员建子房间时能选分类");
    assert_eq!(rooms.category_of(VIEWER, 20).await, Some(category_id(60)));

    rooms.clear_events();
    let missing = rooms
        .service
        .create_sub_room(CreateSubRoom {
            category_id: Some(category_id(99)),
            ..Rooms::request(MEMBER, 21, "website")
        })
        .await;
    assert_eq!(refusal(missing), sub_room(SubRoomRefusal::CategoryMissing));
    assert!(
        !rooms.events().contains(&"matrix.create"),
        "建 Matrix 房间之前就说"
    );

    rooms.delete_category(OWNER, 60).await.expect("删了分类");
    assert_eq!(
        rooms.category_of(VIEWER, 20).await,
        None,
        "回到未分类，子房间还在"
    );
}

#[tokio::test]
async fn 管得了这一间的人能挪分类_能挪回未分类_挪进没有的分类不行_别人挪不了() {
    let rooms = Rooms::new();
    rooms
        .create_category(OWNER, 60, "Release")
        .await
        .expect("建了分类");
    rooms
        .create(MEMBER, 20, "ship")
        .await
        .expect("成员建的，他管这一间");
    rooms.join(VIEWER, 20).await.expect("能进");

    rooms
        .move_to_category(MEMBER, 20, Some(60))
        .await
        .expect("这一间的管理员能挪");
    assert_eq!(rooms.category_of(VIEWER, 20).await, Some(category_id(60)));
    assert_eq!(
        refusal(rooms.move_to_category(VIEWER, 20, None).await),
        sub_room(SubRoomRefusal::ManagersOnly),
        "里面的人不都能挪"
    );
    assert_eq!(
        refusal(rooms.move_to_category(ADMIN, 20, Some(99)).await),
        sub_room(SubRoomRefusal::CategoryMissing),
    );
    assert_eq!(rooms.category_of(VIEWER, 20).await, Some(category_id(60)));
    rooms
        .move_to_category(ADMIN, 20, None)
        .await
        .expect("父房间的管理员没进来也能挪回未分类");
    assert_eq!(rooms.category_of(VIEWER, 20).await, None);

    let renamed = rooms
        .revise(MEMBER, 20, Some("shipping"), None)
        .await
        .expect("只改名");
    assert_eq!(renamed.catalog().name(), "shipping");
    assert_eq!(rooms.category_of(VIEWER, 20).await, None, "没提分类就不动");
}
