//! 房主删除了账户（`specs/redesign/room-roles.md` 的“房主不在了（R6）”）：他名下的房间交给当管理员
//! 最久的那位，先在房间里写一条状态再存；他当管理员的房间记成离开。身份变动都记成没有动手的人。

use agent_room_application::{
    ports::{
        OwnerHandoverWrite, PrivateRoomOwnerHandoverGateway, RoomRoleChangeLog,
        RoomRoleChangeRecord,
    },
    private_rooms::{
        DeletedOwnerRooms, OwnerHandoverDependencies, OwnerHandoverFailure, OwnerHandoverOutcome,
        OwnerHandoverService,
    },
};
use agent_room_domain::{
    owner_handover::AdminTenure,
    room_role_changes::{RoomStanding, RoomStandingChange},
};

use super::{
    public_creation::{bind_room, list_room},
    *,
};

const FORMER_OWNER: &str = "@owner:matrix.test";

/// 写进房间的“交给了谁”。写的时候往事件里记一笔，好看先后。
struct Announcements {
    events: Arc<Mutex<Vec<&'static str>>>,
    writes: Mutex<Vec<(MatrixRoomId, OwnerHandoverWrite)>>,
    unavailable: AtomicBool,
}

impl PrivateRoomOwnerHandoverGateway for Announcements {
    fn announce_owner_handover<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        handover: &'a OwnerHandoverWrite,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            if self.unavailable.load(Ordering::SeqCst) {
                return Err(MatrixFailure::new(
                    MatrixOperation::SendStateEvent,
                    MatrixFailureKind::DependencyUnavailable,
                ));
            }
            self.events.lock().unwrap().push("matrix.handover");
            self.writes
                .lock()
                .unwrap()
                .push((room_id.clone(), handover.clone()));
            Ok(())
        })
    }
}

/// 只答“当了多久”：都没有记录，按成员记录的先后。
struct NoTenureRecords;

impl RoomRoleChangeLog for NoTenureRecords {
    fn recent_role_changes(
        &self,
        _catalog_id: RoomCatalogId,
        _limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomRoleChangeRecord>>> {
        Box::pin(async { unreachable!("交房间不读记录") })
    }

    fn admin_tenure(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AdminTenure>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn handover_service(fixture: &Fixture) -> (OwnerHandoverService, Arc<Announcements>) {
    let announcements = Arc::new(Announcements {
        events: fixture.events.clone(),
        writes: Mutex::new(Vec::new()),
        unavailable: AtomicBool::new(false),
    });
    let principals = Arc::new(TestPrincipalDirectory::new(BTreeMap::from([
        (
            fixture.owner,
            MatrixUserId::new(FORMER_OWNER).expect("用户有效"),
        ),
        (fixture.member, Fixture::member_matrix()),
    ])));
    let service = OwnerHandoverService::new(OwnerHandoverDependencies {
        store: fixture.store.clone(),
        listings: fixture.listings.clone(),
        role_changes: Arc::new(NoTenureRecords),
        principals,
        matrix: fixture.matrix.clone(),
        announcements: announcements.clone(),
        clock: Arc::new(TestRuntime),
    });
    (service, announcements)
}

async fn hand_over(
    service: &OwnerHandoverService,
    principal_id: PrincipalId,
    matrix_user_id: &str,
) -> Result<OwnerHandoverOutcome, OwnerHandoverFailure> {
    service
        .hand_over_rooms_of(
            principal_id,
            &MatrixUserId::new(matrix_user_id).expect("用户有效"),
        )
        .await
}

fn stored(fixture: &Fixture) -> PrivateRoomSnapshot {
    fixture.store.snapshot.lock().unwrap().clone().unwrap()
}

fn recorded(fixture: &Fixture) -> Vec<RoomRoleChange> {
    std::mem::take(&mut *fixture.store.role_changes.lock().unwrap())
}

fn admin() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
        PrivateRoomCapability::Manage,
    ])
    .expect("管理员权限有效")
}

/// 有“管理”、没有“发言”：领域规则允许，接手时要补上发言级别。
fn silent_admin() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Manage,
    ])
    .expect("不说话的管理员权限有效")
}

fn standing(
    status: PrivateRoomMembershipStatus,
    owner: bool,
    permissions: PrivateRoomPermissions,
) -> RoomStanding {
    RoomStanding {
        status,
        owner,
        permissions,
    }
}

#[tokio::test]
async fn 房间交给管理员_先写状态再存_原房主记成移出_没有动手的人() {
    let fixture = Fixture::joined(admin()).await;
    let (service, announcements) = handover_service(&fixture);
    recorded(&fixture);
    fixture.clear_events();

    let outcome = hand_over(&service, fixture.owner, FORMER_OWNER)
        .await
        .expect("交出去");

    assert_eq!(
        outcome,
        OwnerHandoverOutcome {
            handed_over: 1,
            ..OwnerHandoverOutcome::default()
        }
    );
    assert_eq!(
        fixture.events(),
        ["matrix.handover", "store.save"],
        "先写状态再存；他本来能说话，发言级别不用动"
    );
    let writes = announcements.writes.lock().unwrap().clone();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].1.owner, Fixture::member_matrix());
    assert_eq!(writes[0].1.former_owner.as_str(), FORMER_OWNER);
    let room = stored(&fixture);
    assert_eq!(room.room().owner_principal_id(), fixture.member);
    assert_eq!(
        fixture.store.member_status(fixture.owner),
        PrivateRoomMembershipStatus::Removed
    );
    let changes = recorded(&fixture);
    assert!(changes.iter().all(|change| change.actor.is_none()));
    assert!(changes.contains(&RoomRoleChange {
        actor: None,
        change: RoomStandingChange {
            target: fixture.member,
            previous: Some(standing(
                PrivateRoomMembershipStatus::Joined,
                false,
                admin()
            )),
            next: standing(
                PrivateRoomMembershipStatus::Joined,
                true,
                PrivateRoomPermissions::ALL
            ),
        },
    }));

    // 再来一遍什么都不做：他已经不在这间房的名单里了。
    fixture.clear_events();
    assert_eq!(
        hand_over(&service, fixture.owner, FORMER_OWNER).await,
        Ok(OwnerHandoverOutcome::default())
    );
    assert!(fixture.events().is_empty());
}

#[tokio::test]
async fn 接手的管理员原来不能说话_存之前补上发言级别() {
    let fixture = Fixture::joined(silent_admin()).await;
    let (service, _) = handover_service(&fixture);
    fixture.clear_events();

    hand_over(&service, fixture.owner, FORMER_OWNER)
        .await
        .expect("交出去");

    assert_eq!(
        fixture.events(),
        ["matrix.handover", "matrix.speak.on", "store.save"]
    );
    assert_eq!(stored(&fixture).room().owner_principal_id(), fixture.member);
}

#[tokio::test]
async fn 没有管理员的房间不动_留给匿名化归档() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    let (service, announcements) = handover_service(&fixture);
    fixture.clear_events();

    let outcome = hand_over(&service, fixture.owner, FORMER_OWNER)
        .await
        .expect("没有可交的也算做完");

    assert_eq!(
        outcome,
        OwnerHandoverOutcome {
            without_successor: 1,
            ..OwnerHandoverOutcome::default()
        }
    );
    assert!(fixture.events().is_empty());
    assert!(announcements.writes.lock().unwrap().is_empty());
    assert_eq!(stored(&fixture).room().owner_principal_id(), fixture.owner);
}

#[tokio::test]
async fn 公开房间轮不到名下已经有一间公开房间的管理员() {
    let fixture = Fixture::joined(admin()).await;
    list_room(&fixture);
    *fixture.listings.owner_listed_elsewhere.lock().unwrap() = true;
    let (service, announcements) = handover_service(&fixture);
    fixture.clear_events();

    let outcome = hand_over(&service, fixture.owner, FORMER_OWNER)
        .await
        .expect("轮不上也算做完");

    assert_eq!(outcome.without_successor, 1);
    assert!(announcements.writes.lock().unwrap().is_empty());
    assert_eq!(stored(&fixture).room().owner_principal_id(), fixture.owner);
}

#[tokio::test]
async fn 绑着仓库的公开房间照样交给名下已有公开房间的管理员() {
    let fixture = Fixture::joined(admin()).await;
    list_room(&fixture);
    bind_room(&fixture);
    *fixture.listings.owner_listed_elsewhere.lock().unwrap() = true;
    let (service, _) = handover_service(&fixture);

    let outcome = hand_over(&service, fixture.owner, FORMER_OWNER)
        .await
        .expect("交得出去");

    assert_eq!(outcome.handed_over, 1, "绑着仓库的不占名额");
    assert_eq!(stored(&fixture).room().owner_principal_id(), fixture.member);
}

#[tokio::test]
async fn 存的时候才撞上公开房间的限额_收回补上的发言级别() {
    let fixture = Fixture::joined(silent_admin()).await;
    list_room(&fixture);
    *fixture.listings.limit_on_save.lock().unwrap() = true;
    let (service, _) = handover_service(&fixture);
    fixture.clear_events();

    let outcome = hand_over(&service, fixture.owner, FORMER_OWNER)
        .await
        .expect("轮不上也算做完");

    assert_eq!(outcome.without_successor, 1);
    assert_eq!(
        fixture.events(),
        ["matrix.handover", "matrix.speak.on", "matrix.speak.off"]
    );
    assert_eq!(stored(&fixture).room().owner_principal_id(), fixture.owner);
}

#[tokio::test]
async fn 他在别人房间里当管理员_记成离开_没有动手的人() {
    let fixture = Fixture::joined(admin()).await;
    let (service, announcements) = handover_service(&fixture);
    recorded(&fixture);
    fixture.clear_events();

    let outcome = hand_over(&service, fixture.member, "@member:matrix.test")
        .await
        .expect("记成离开");

    assert_eq!(
        outcome,
        OwnerHandoverOutcome {
            left_as_admin: 1,
            ..OwnerHandoverOutcome::default()
        }
    );
    assert_eq!(
        fixture.events(),
        ["store.save"],
        "他的聊天账户已经停用，聊天服务器上不用动"
    );
    assert!(announcements.writes.lock().unwrap().is_empty());
    assert_eq!(
        fixture.store.member_status(fixture.member),
        PrivateRoomMembershipStatus::Removed
    );
    assert_eq!(
        recorded(&fixture),
        [RoomRoleChange {
            actor: None,
            change: RoomStandingChange {
                target: fixture.member,
                previous: Some(standing(
                    PrivateRoomMembershipStatus::Joined,
                    false,
                    admin()
                )),
                next: standing(
                    PrivateRoomMembershipStatus::Removed,
                    false,
                    PrivateRoomPermissions::NONE
                ),
            },
        }]
    );
}

#[tokio::test]
async fn 普通成员的房间不归这里管() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    let (service, _) = handover_service(&fixture);
    fixture.clear_events();

    assert_eq!(
        hand_over(&service, fixture.member, "@member:matrix.test").await,
        Ok(OwnerHandoverOutcome::default())
    );
    assert!(fixture.events().is_empty());
}

#[tokio::test]
async fn 写状态没成_什么都不存_删除任务重来() {
    let fixture = Fixture::joined(admin()).await;
    let (service, announcements) = handover_service(&fixture);
    announcements.unavailable.store(true, Ordering::SeqCst);
    fixture.clear_events();

    assert_eq!(
        hand_over(&service, fixture.owner, FORMER_OWNER).await,
        Err(OwnerHandoverFailure::MATRIX)
    );
    assert!(fixture.events().is_empty());
    assert_eq!(stored(&fixture).room().owner_principal_id(), fixture.owner);
}
