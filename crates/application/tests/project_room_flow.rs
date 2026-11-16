//! 项目房间的认领、核对、绑定（`specs/redesign/project-rooms.md` 的 3P-2b）：同一个人同一个仓库再认领
//! 回同一个、最多 3 个在等；拿已有的房间认领只能是自己名下公开着的；核对时找到那一行就绑上（没带房间的
//! 替他建一间）；原来那间房的链接还在就一直等，不在了绑定挪过来；读不了的照实记下；后台一轮作废 7 天
//! 没认上的、GitHub 停着时不读；房主解绑、再核对一次。

#[path = "project_room_flow/fakes.rs"]
mod fakes;

use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    ports::{
        CreateProjectRoom, PortFuture, PrincipalAccount, ProjectRoomCandidate, ProjectRoomCreation,
        PublicRoomRecord, RepositoryBindingRecord, RepositoryReadFailure,
    },
    private_rooms::PrivateRoomActor,
    project_rooms::{
        BindingCheckResult, ClaimRepository, ClaimStatus, ClaimView, InspectClaim,
        InspectRoomRepository, ProjectRoomClaimChecks, ProjectRoomDependencies,
        ProjectRoomFailureKind, ProjectRoomService, ProjectRoomUseCases, RepositoryRoomPageService,
        RepositoryRoomPages,
    },
    public_room_directory::{
        BrowsePublicRooms, PublicDirectoryFailure, PublicDirectoryFailureKind,
        PublicDirectoryResult, PublicRoomDetailView, PublicRoomDirectoryUseCases, PublicRoomOnline,
        PublicRoomPage, PublicRoomView,
    },
};
use agent_room_domain::{
    identity::Principal,
    ids::{DeviceId, PrincipalId, RoomCatalogId},
    project_rooms::{ClaimCheckResult, GithubRepository, RepositoryFile},
    room_listing::{RoomJoinMode, RoomShareCode},
    time::UtcMillis,
};
use fakes::{
    CountingSecrets, MemoryStore, RecordingCreator, ScriptedReader, TestClock, TestIdentifiers,
};
use uuid::Uuid;

const NOW: i64 = 1_790_000_000_000;
const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

struct Fixture {
    service: ProjectRoomService,
    store: Arc<MemoryStore>,
    reader: Arc<ScriptedReader>,
    creator: Arc<RecordingCreator>,
    clock: Arc<TestClock>,
}

impl Fixture {
    fn new() -> Self {
        let store = Arc::new(MemoryStore::default());
        let reader = Arc::new(ScriptedReader::default());
        let creator = Arc::new(RecordingCreator {
            store: store.clone(),
            requests: Mutex::new(Vec::new()),
            failure: Mutex::new(None),
            next_room: created_room(),
        });
        let clock = Arc::new(TestClock(Mutex::new(NOW)));
        let service = ProjectRoomService::new(ProjectRoomDependencies {
            store: store.clone(),
            reader: reader.clone(),
            creator: creator.clone(),
            secrets: Arc::new(CountingSecrets::default()),
            identifiers: Arc::new(TestIdentifiers),
            clock: clock.clone(),
        });
        Self {
            service,
            store,
            reader,
            creator,
            clock,
        }
    }

    async fn claim(&self, who: PrincipalId, repository: &str) -> ClaimView {
        self.service
            .claim(claim(who, repository, None))
            .await
            .expect("认领得了")
    }

    async fn check(&self, who: PrincipalId, view: &ClaimView) -> ClaimView {
        self.service
            .check_claim(inspect_claim(who, view))
            .await
            .expect("核对得了")
    }

    /// 他名下一间公开着的房间。
    fn public_room(&self, catalog_id: RoomCatalogId, owner: PrincipalId, code: &str) {
        self.store.add_room(ProjectRoomCandidate {
            catalog_id,
            owner: Some(owner),
            active: true,
            sub_room: false,
            listed: true,
            share_code: Some(share_code(code)),
            held: false,
            bound: None,
        });
    }

    fn bind(&self, catalog_id: RoomCatalogId, repository: &str, by: PrincipalId) {
        self.store.bind(RepositoryBindingRecord {
            catalog_id,
            repository: GithubRepository::parse(repository).unwrap(),
            verified_by: by,
            verified_at: time(NOW - DAY),
            verified_file: RepositoryFile::AgentsMd,
        });
    }
}

#[tokio::test]
async fn 同一个人同一个仓库再认领回同一个_最多三个在等() {
    let fixture = Fixture::new();
    let first = fixture.claim(owner(), "octo/hello").await;
    assert_eq!(first.status, ClaimStatus::Pending);
    assert_eq!(first.claimed_at, Some(time(NOW)));
    assert_eq!(first.catalog_id, None);
    assert_eq!(first.last_check, None);
    let again = fixture.claim(owner(), "OCTO/Hello").await;
    assert_eq!(again.claim_id, first.claim_id, "大小写不同也是同一个仓库");
    assert_eq!(again.share_code, first.share_code);

    let second = fixture.claim(owner(), "octo/world").await;
    assert_ne!(second.share_code, first.share_code, "每个认领一个码");
    fixture.claim(owner(), "octo/third").await;
    let refused = fixture
        .service
        .claim(claim(owner(), "octo/fourth", None))
        .await
        .unwrap_err();
    assert_eq!(refused.kind(), ProjectRoomFailureKind::TooManyClaims);
    fixture.claim(stranger(), "octo/fourth").await;

    for bad in [
        "octo",
        "octo/hello/extra",
        "-octo/hello",
        "octo/..",
        "octo/hel lo",
    ] {
        let refused = fixture
            .service
            .claim(claim(owner(), bad, None))
            .await
            .unwrap_err();
        assert_eq!(
            refused.kind(),
            ProjectRoomFailureKind::InvalidRepository,
            "{bad}"
        );
    }

    let mut expired = principal(owner());
    expired.expires_at = time(NOW);
    let refused = fixture
        .service
        .claim(ClaimRepository {
            actor: PrivateRoomActor::Person(expired),
            repository: "octo/other".to_owned(),
            target: None,
        })
        .await
        .unwrap_err();
    assert_eq!(refused.kind(), ProjectRoomFailureKind::Forbidden);

    fixture
        .service
        .cancel_claim(inspect_claim(owner(), &first))
        .await
        .expect("撤得掉");
    assert_eq!(
        fixture
            .service
            .inspect_claim(inspect_claim(owner(), &first))
            .await
            .unwrap_err()
            .kind(),
        ProjectRoomFailureKind::NotFound
    );
    fixture.claim(owner(), "octo/fourth").await;
}

#[tokio::test]
async fn 本机设备替账户认领和网页的是同一个_网络_agent_一律不让() {
    let fixture = Fixture::new();
    let by_web = fixture.claim(owner(), "octo/hello").await;
    let by_device = fixture
        .service
        .claim(ClaimRepository {
            actor: PrivateRoomActor::Device(device(owner())),
            repository: "octo/hello".to_owned(),
            target: None,
        })
        .await
        .expect("本机设备认领得了");
    assert_eq!(by_device.claim_id, by_web.claim_id, "同一个账户同一个仓库");
    let checked = fixture
        .service
        .check_claim(InspectClaim {
            actor: PrivateRoomActor::Device(device(owner())),
            claim_id: by_device.claim_id.unwrap(),
        })
        .await
        .expect("本机设备核对得了");
    assert_eq!(checked.status, ClaimStatus::Pending);

    let refused = fixture
        .service
        .claim(ClaimRepository {
            actor: PrivateRoomActor::NetworkAgent(device(stranger())),
            repository: "octo/world".to_owned(),
            target: None,
        })
        .await
        .unwrap_err();
    assert_eq!(refused.kind(), ProjectRoomFailureKind::Forbidden);
    let refused = fixture
        .service
        .check_claim(InspectClaim {
            actor: PrivateRoomActor::NetworkAgent(device(owner())),
            claim_id: by_device.claim_id.unwrap(),
        })
        .await
        .unwrap_err();
    assert_eq!(refused.kind(), ProjectRoomFailureKind::Forbidden);
}

#[tokio::test]
async fn 拿已有的房间认领_只能是自己名下公开着_没绑别的仓库的() {
    let fixture = Fixture::new();
    fixture.public_room(room(1), owner(), "Room01");
    let mut unlisted = public_room_candidate(room(2), owner(), "Room02");
    unlisted.listed = false;
    fixture.store.add_room(unlisted);
    fixture.public_room(room(3), owner(), "Room03");
    fixture.bind(room(3), "octo/other", owner());

    for (target, kind) in [
        (room(1), ProjectRoomFailureKind::Forbidden),
        (room(9), ProjectRoomFailureKind::NotFound),
    ] {
        let refused = fixture
            .service
            .claim(claim(stranger(), "octo/hello", Some(target)))
            .await
            .unwrap_err();
        assert_eq!(refused.kind(), kind);
    }
    for (target, kind) in [
        (room(2), ProjectRoomFailureKind::RoomNotEligible),
        (room(3), ProjectRoomFailureKind::RoomAlreadyBound),
    ] {
        let refused = fixture
            .service
            .claim(claim(owner(), "octo/hello", Some(target)))
            .await
            .unwrap_err();
        assert_eq!(refused.kind(), kind);
    }

    let view = fixture
        .service
        .claim(claim(owner(), "octo/hello", Some(room(1))))
        .await
        .expect("自己名下公开着的房间认领得了");
    assert_eq!(view.share_code, share_code("Room01"), "码就是房间的分享码");
    assert_eq!(view.catalog_id, Some(room(1)));
}

#[tokio::test]
async fn 带房间的认领找到那一行就绑上_一分钟里只能核对一次() {
    let fixture = Fixture::new();
    fixture.public_room(room(1), owner(), "Room01");
    let view = fixture
        .service
        .claim(claim(owner(), "octo/hello", Some(room(1))))
        .await
        .unwrap();

    let pending = fixture.check(owner(), &view).await;
    assert_eq!(pending.status, ClaimStatus::Pending);
    let last = pending.last_check.unwrap();
    assert_eq!(
        (last.at, last.result),
        (time(NOW), ClaimCheckResult::NotInRepository)
    );

    fixture.reader.write_line(
        "octo/hello",
        &share_code("Room01"),
        RepositoryFile::ReadmeMd,
    );
    fixture.clock.advance(30_000);
    let too_soon = fixture
        .service
        .check_claim(inspect_claim(owner(), &view))
        .await
        .unwrap_err();
    assert_eq!(too_soon.kind(), ProjectRoomFailureKind::CheckTooSoon);
    assert_eq!(too_soon.retry_at(), Some(time(NOW + MINUTE)));
    let refused = fixture
        .service
        .check_claim(inspect_claim(stranger(), &view))
        .await
        .unwrap_err();
    assert_eq!(
        refused.kind(),
        ProjectRoomFailureKind::NotFound,
        "别人的认领看不到"
    );

    fixture.clock.advance(30_000);
    let bound = fixture.check(owner(), &view).await;
    assert_eq!(bound.status, ClaimStatus::Bound);
    assert_eq!(bound.claim_id, None);
    assert_eq!(bound.catalog_id, Some(room(1)));
    let binding = fixture.store.binding_of(room(1)).expect("绑上了");
    assert_eq!(binding.verified_by, owner());
    assert_eq!(binding.verified_at, time(NOW + MINUTE));
    assert_eq!(binding.verified_file, RepositoryFile::ReadmeMd);
    assert!(fixture.store.claims.lock().unwrap().is_empty(), "认上就删");
    assert!(
        fixture.creator.requests.lock().unwrap().is_empty(),
        "已有的房间不另建"
    );

    let again = fixture
        .service
        .claim(claim(owner(), "octo/hello", None))
        .await
        .unwrap();
    assert_eq!(
        again.status,
        ClaimStatus::Bound,
        "已经绑在自己的房间上，直接回那间房"
    );
    assert_eq!(again.catalog_id, Some(room(1)));
    assert!(fixture.store.claims.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 没带房间的认领_认上以后替他建一间项目房间() {
    let fixture = Fixture::new();
    let view = fixture.claim(owner(), "octo/hello").await;
    fixture
        .reader
        .write_line("octo/hello", &view.share_code, RepositoryFile::AgentsMd);
    let bound = fixture.check(owner(), &view).await;
    assert_eq!(bound.status, ClaimStatus::Bound);
    assert_eq!(bound.catalog_id, Some(created_room()));
    assert_eq!(
        bound.share_code, view.share_code,
        "新房间的分享码就是认领的码"
    );
    assert_eq!(
        *fixture.creator.requests.lock().unwrap(),
        [CreateProjectRoom {
            claim_id: view.claim_id.unwrap(),
            owner: owner(),
            repository: GithubRepository::parse("octo/hello").unwrap(),
            share_code: view.share_code.clone(),
            verified_at: time(NOW),
            verified_file: RepositoryFile::AgentsMd,
            replacing: None,
        }]
    );
    assert_eq!(
        fixture
            .store
            .binding_of(created_room())
            .unwrap()
            .verified_by,
        owner()
    );
}

#[tokio::test]
async fn 原来那间房的链接还在就一直等_不在了绑定挪过来() {
    let fixture = Fixture::new();
    fixture.public_room(room(1), stranger(), "OldRm1");
    fixture.bind(room(1), "octo/hello", stranger());
    fixture.reader.write_line(
        "octo/hello",
        &share_code("OldRm1"),
        RepositoryFile::AgentsMd,
    );

    let view = fixture.claim(owner(), "octo/hello").await;
    fixture
        .reader
        .write_line("octo/hello", &view.share_code, RepositoryFile::ReadmeMd);
    let waiting = fixture.check(owner(), &view).await;
    assert_eq!(
        waiting.last_check.unwrap().result,
        ClaimCheckResult::RepositoryTaken,
        "两行都在"
    );
    assert_eq!(
        fixture.reader.scans.lock().unwrap()[0].1,
        [view.share_code.clone(), share_code("OldRm1")],
        "一次读两个码"
    );

    fixture
        .reader
        .remove_line("octo/hello", &share_code("OldRm1"));
    fixture.clock.advance(MINUTE);
    let bound = fixture.check(owner(), &view).await;
    assert_eq!(bound.catalog_id, Some(created_room()));
    assert_eq!(
        fixture.creator.requests.lock().unwrap()[0].replacing,
        Some(room(1)),
        "原来那间房的绑定挪过来"
    );
    assert_eq!(fixture.store.binding_of(room(1)), None);
    assert!(fixture.store.binding_of(created_room()).is_some());
}

#[tokio::test]
async fn 平台拿掉的房间占着仓库_别人认领不了() {
    let fixture = Fixture::new();
    let mut held = public_room_candidate(room(1), stranger(), "OldRm1");
    held.held = true;
    fixture.store.add_room(held);
    fixture.bind(room(1), "octo/hello", stranger());
    let refused = fixture
        .service
        .claim(claim(owner(), "octo/hello", None))
        .await
        .unwrap_err();
    assert_eq!(refused.kind(), ProjectRoomFailureKind::RepositoryTaken);
}

#[tokio::test]
async fn 读不了的和建不成的照实记下_认领留着() {
    for (setup, expected) in [
        (
            Setup::Fail(RepositoryReadFailure::Renamed),
            ClaimCheckResult::RepositoryRenamed,
        ),
        (
            Setup::Fail(RepositoryReadFailure::Unavailable { retry_at: None }),
            ClaimCheckResult::GithubUnavailable,
        ),
        (Setup::Truncated, ClaimCheckResult::FileTooLarge),
        (Setup::CreationFails, ClaimCheckResult::CreationFailed),
    ] {
        let fixture = Fixture::new();
        let view = fixture.claim(owner(), "octo/hello").await;
        match setup {
            Setup::Fail(failure) => *fixture.reader.failure.lock().unwrap() = Some(failure),
            Setup::Truncated => *fixture.reader.truncated.lock().unwrap() = true,
            Setup::CreationFails => {
                fixture
                    .reader
                    .write_line("octo/hello", &view.share_code, RepositoryFile::AgentsMd);
                *fixture.creator.failure.lock().unwrap() = Some(ProjectRoomCreation::Failed);
            }
        }
        let pending = fixture.check(owner(), &view).await;
        assert_eq!(pending.status, ClaimStatus::Pending);
        assert_eq!(pending.last_check.unwrap().result, expected);
        let saved = fixture
            .store
            .claim_of(view.claim_id.unwrap())
            .expect("认领留着");
        assert_eq!(saved.last_check.unwrap().result, expected);
        assert_eq!(saved.check_count, 1);
    }
}

#[derive(Clone, Copy)]
enum Setup {
    Fail(RepositoryReadFailure),
    Truncated,
    CreationFails,
}

#[tokio::test]
async fn 后台一轮作废七天没认上的_头一天五分钟一次_之后一小时一次() {
    let fixture = Fixture::new();
    let old = fixture.claim(owner(), "octo/old").await;
    fixture.clock.advance(6 * DAY);
    let middle = fixture.claim(owner(), "octo/middle").await;
    fixture.clock.advance(DAY - MINUTE);
    let young = fixture.claim(stranger(), "octo/young").await;

    let round = fixture.service.check_due().await.unwrap();
    assert_eq!(round.expired, 0);
    assert_eq!(round.checked, 2, "头一天的那个刚认领还不到五分钟");
    assert_eq!(round.pending, 2);
    assert_eq!(fixture.reader.scan_count(), 2);

    fixture.clock.advance(5 * MINUTE);
    let round = fixture.service.check_due().await.unwrap();
    assert_eq!(round.expired, 1, "满了七天的作废");
    assert!(fixture.store.claim_of(old.claim_id.unwrap()).is_none());
    assert_eq!(round.checked, 1, "只有头一天的那个又到了五分钟");
    let young_record = fixture.store.claim_of(young.claim_id.unwrap()).unwrap();
    assert_eq!(young_record.check_count, 1);
    let middle_record = fixture.store.claim_of(middle.claim_id.unwrap()).unwrap();
    assert_eq!(middle_record.check_count, 1, "过了头一天的一小时一次");

    fixture.clock.advance(HOUR);
    fixture
        .reader
        .write_line("octo/young", &young.share_code, RepositoryFile::AgentsMd);
    *fixture.reader.paused_until.lock().unwrap() = Some(time(NOW + 8 * DAY));
    let round = fixture.service.check_due().await.unwrap();
    assert!(round.deferred, "GitHub 停着时这一轮不读");
    assert_eq!(round.checked, 0);
    assert_eq!(fixture.reader.scan_count(), 3);

    *fixture.reader.paused_until.lock().unwrap() = None;
    let round = fixture.service.check_due().await.unwrap();
    assert_eq!((round.checked, round.bound, round.pending), (2, 1, 1));
    assert!(fixture.store.claim_of(young.claim_id.unwrap()).is_none());
}

#[tokio::test]
async fn 带房间的认领_房间不公开了就删掉认领() {
    let fixture = Fixture::new();
    fixture.public_room(room(1), owner(), "Room01");
    let view = fixture
        .service
        .claim(claim(owner(), "octo/hello", Some(room(1))))
        .await
        .unwrap();
    fixture.reader.write_line(
        "octo/hello",
        &share_code("Room01"),
        RepositoryFile::AgentsMd,
    );
    fixture
        .store
        .rooms
        .lock()
        .unwrap()
        .get_mut(&room(1))
        .unwrap()
        .listed = false;
    let dropped = fixture
        .service
        .check_claim(inspect_claim(owner(), &view))
        .await
        .unwrap_err();
    assert_eq!(dropped.kind(), ProjectRoomFailureKind::RoomNotEligible);
    assert!(fixture.store.claims.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 房主看_解绑_名下另有一间不绑仓库的公开房间时不让解绑() {
    let fixture = Fixture::new();
    fixture.public_room(room(1), owner(), "Room01");
    fixture.bind(room(1), "octo/hello", owner());
    fixture.public_room(room(2), owner(), "Room02");

    let view = fixture
        .service
        .binding(inspect_room(owner(), room(1)))
        .await
        .unwrap()
        .expect("绑着");
    assert_eq!(view.binding.repository.full_name(), "octo/hello");
    assert_eq!(view.share_code, Some(share_code("Room01")));
    assert_eq!(
        fixture
            .service
            .binding(inspect_room(owner(), room(2)))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        fixture
            .service
            .binding(inspect_room(stranger(), room(1)))
            .await
            .unwrap_err()
            .kind(),
        ProjectRoomFailureKind::Forbidden,
        "只有房主看得到"
    );

    let refused = fixture
        .service
        .unbind(inspect_room(owner(), room(1)))
        .await
        .unwrap_err();
    assert_eq!(refused.kind(), ProjectRoomFailureKind::LimitReached);
    assert!(fixture.store.binding_of(room(1)).is_some());

    fixture
        .store
        .rooms
        .lock()
        .unwrap()
        .get_mut(&room(2))
        .unwrap()
        .listed = false;
    fixture
        .service
        .unbind(inspect_room(owner(), room(1)))
        .await
        .expect("另一间改回私密以后解得了");
    assert_eq!(fixture.store.binding_of(room(1)), None);
    fixture
        .service
        .unbind(inspect_room(owner(), room(1)))
        .await
        .expect("没绑着也算解了");
}

#[tokio::test]
async fn 列出自己在等的认领_别人的不在里面_认上了就没了() {
    let fixture = Fixture::new();
    fixture.public_room(room(1), owner(), "Room01");
    let first = fixture.claim(owner(), "octo/hello").await;
    let second = fixture
        .service
        .claim(claim(owner(), "octo/world", Some(room(1))))
        .await
        .unwrap();
    fixture.claim(stranger(), "octo/other").await;

    let mine = fixture
        .service
        .claims(PrivateRoomActor::Person(principal(owner())))
        .await
        .unwrap();
    assert_eq!(mine, [first.clone(), second.clone()], "最早的在前");
    assert_eq!(mine[1].catalog_id, Some(room(1)));

    fixture
        .reader
        .write_line("octo/world", &second.share_code, RepositoryFile::ReadmeMd);
    fixture.check(owner(), &second).await;
    let mine = fixture
        .service
        .claims(PrivateRoomActor::Device(device(owner())))
        .await
        .unwrap();
    assert_eq!(mine, [first], "认上的那个不在了");
}

#[tokio::test]
async fn 房间绑着哪个仓库谁都能问_只认公开着的主房间() {
    let fixture = Fixture::new();
    fixture.public_room(room(1), owner(), "Room01");
    fixture.bind(room(1), "octo/hello", owner());
    fixture.public_room(room(2), owner(), "Room02");
    let ask = |catalog_id| {
        fixture
            .service
            .room_repository(inspect_room(stranger(), catalog_id))
    };

    assert_eq!(
        ask(room(1)).await.unwrap(),
        Some(GithubRepository::parse("octo/hello").unwrap()),
        "不是房主也看得到"
    );
    assert_eq!(ask(room(2)).await.unwrap(), None, "没绑");
    assert_eq!(ask(room(9)).await.unwrap(), None, "没有这间房也只是 None");

    for change in [
        |room: &mut ProjectRoomCandidate| room.listed = false,
        |room: &mut ProjectRoomCandidate| room.active = false,
        |room: &mut ProjectRoomCandidate| room.held = true,
        |room: &mut ProjectRoomCandidate| room.share_code = None,
    ] {
        let saved = fixture.store.rooms.lock().unwrap()[&room(1)].clone();
        change(
            fixture
                .store
                .rooms
                .lock()
                .unwrap()
                .get_mut(&room(1))
                .unwrap(),
        );
        assert_eq!(ask(room(1)).await.unwrap(), None);
        fixture.store.add_room(saved);
    }
    assert!(ask(room(1)).await.unwrap().is_some());
}

#[tokio::test]
async fn 房主再核对一次_找到了记下这次_没找到照实说不解绑() {
    let fixture = Fixture::new();
    fixture.public_room(room(1), owner(), "Room01");
    fixture.bind(room(1), "octo/hello", stranger());
    fixture.reader.write_line(
        "octo/hello",
        &share_code("Room01"),
        RepositoryFile::ReadmeMd,
    );

    let checked = fixture
        .service
        .check_binding(inspect_room(owner(), room(1)))
        .await
        .unwrap();
    assert_eq!(
        checked.result,
        BindingCheckResult::Found(RepositoryFile::ReadmeMd)
    );
    assert_eq!(checked.checked_at, time(NOW));
    let binding = fixture.store.binding_of(room(1)).unwrap();
    assert_eq!(
        (
            binding.verified_by,
            binding.verified_at,
            binding.verified_file
        ),
        (owner(), time(NOW), RepositoryFile::ReadmeMd)
    );

    fixture.clock.advance(MINUTE - 1);
    let too_soon = fixture
        .service
        .check_binding(inspect_room(owner(), room(1)))
        .await
        .unwrap_err();
    assert_eq!(too_soon.kind(), ProjectRoomFailureKind::CheckTooSoon);
    assert_eq!(too_soon.retry_at(), Some(time(NOW + MINUTE)));

    fixture.clock.advance(1);
    fixture
        .reader
        .remove_line("octo/hello", &share_code("Room01"));
    let missing = fixture
        .service
        .check_binding(inspect_room(owner(), room(1)))
        .await
        .unwrap();
    assert_eq!(
        missing.result,
        BindingCheckResult::Missing(ClaimCheckResult::NotInRepository)
    );
    assert_eq!(
        fixture.store.binding_of(room(1)).unwrap().verified_at,
        time(NOW),
        "没找到不改记录、不解绑"
    );
}

/// 发现页的目录：只认 `codes` 里的分享码，记下问过哪些。
#[derive(Default)]
struct CodeDirectory {
    rooms: Mutex<Vec<(String, RoomCatalogId)>>,
    asked: Mutex<Vec<String>>,
}

impl PublicRoomDirectoryUseCases for CodeDirectory {
    fn browse(
        &self,
        _request: BrowsePublicRooms,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomPage>> {
        Box::pin(async { unreachable!("仓库地址不翻目录") })
    }

    fn detail(
        &self,
        viewer: Option<AuthenticatedPrincipal>,
        share_code: String,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomDetailView>> {
        assert!(viewer.is_none(), "仓库地址不看登录");
        self.asked.lock().unwrap().push(share_code.clone());
        let found = self
            .rooms
            .lock()
            .unwrap()
            .iter()
            .find(|(code, _)| *code == share_code)
            .map(|(code, catalog_id)| listed_detail(*catalog_id, code));
        Box::pin(async move {
            found.ok_or(PublicDirectoryFailure::new(
                "test",
                PublicDirectoryFailureKind::NotFound,
            ))
        })
    }
}

#[tokio::test]
async fn 仓库地址只认绑着它_此刻公开着的那间房_不分没有的是哪一种() {
    let fixture = Fixture::new();
    let directory = Arc::new(CodeDirectory::default());
    let pages = RepositoryRoomPageService::new(fixture.store.clone(), directory.clone());
    let lookup = |repository: &str| {
        let repository = GithubRepository::parse(repository).unwrap();
        let pages = &pages;
        async move { pages.public_room(repository).await }
    };

    // 没绑：不去问目录。
    let missing = lookup("octo/hello").await.unwrap_err();
    assert_eq!(missing.kind(), PublicDirectoryFailureKind::NotFound);
    assert!(directory.asked.lock().unwrap().is_empty());

    // 绑着、公开着：大小写不论，出目录里的那一份。
    fixture.public_room(room(1), owner(), "Proj01");
    fixture.bind(room(1), "octo/hello", owner());
    directory
        .rooms
        .lock()
        .unwrap()
        .push(("Proj01".to_owned(), room(1)));
    let found = lookup("OCTO/Hello").await.unwrap();
    assert_eq!(found.room.room.catalog_id, room(1));
    assert_eq!(
        found.online,
        Some(PublicRoomOnline {
            count: 4,
            at_least: false
        })
    );

    // 目录说不公开了（改回私密、关了）：和没绑一样。
    directory.rooms.lock().unwrap().clear();
    let private = lookup("octo/hello").await.unwrap_err();
    assert_eq!(private.kind(), PublicDirectoryFailureKind::NotFound);

    // 码换给了别的房间：只认绑着仓库的那一间。
    directory
        .rooms
        .lock()
        .unwrap()
        .push(("Proj01".to_owned(), room(2)));
    let moved = lookup("octo/hello").await.unwrap_err();
    assert_eq!(moved.kind(), PublicDirectoryFailureKind::NotFound);

    // 平台拿掉的照旧占着仓库，可不给人看，也不去问目录。
    let mut held = public_room_candidate(room(3), stranger(), "Held01");
    held.held = true;
    fixture.store.add_room(held);
    fixture.bind(room(3), "octo/held", stranger());
    directory
        .rooms
        .lock()
        .unwrap()
        .push(("Held01".to_owned(), room(3)));
    let asked_before = directory.asked.lock().unwrap().len();
    let hidden = lookup("octo/held").await.unwrap_err();
    assert_eq!(hidden.kind(), PublicDirectoryFailureKind::NotFound);
    assert_eq!(directory.asked.lock().unwrap().len(), asked_before);
}

fn listed_detail(catalog_id: RoomCatalogId, code: &str) -> PublicRoomDetailView {
    PublicRoomDetailView {
        room: PublicRoomView {
            room: PublicRoomRecord {
                catalog_id,
                name: "octo/hello".to_owned(),
                description: String::new(),
                owner_display_name: "Someone".to_owned(),
                member_count: 2,
                agent_count: 3,
                join_mode: RoomJoinMode::Open,
                end_to_end_encrypted: false,
                share_code: share_code(code),
                created_at: time(NOW - DAY),
                last_message_at: None,
            },
            viewer: None,
        },
        admin_display_names: Vec::new(),
        online: Some(PublicRoomOnline {
            count: 4,
            at_least: false,
        }),
    }
}

fn claim(who: PrincipalId, repository: &str, target: Option<RoomCatalogId>) -> ClaimRepository {
    ClaimRepository {
        actor: PrivateRoomActor::Person(principal(who)),
        repository: repository.to_owned(),
        target,
    }
}

fn inspect_claim(who: PrincipalId, view: &ClaimView) -> InspectClaim {
    InspectClaim {
        actor: PrivateRoomActor::Person(principal(who)),
        claim_id: view.claim_id.unwrap(),
    }
}

fn inspect_room(who: PrincipalId, catalog_id: RoomCatalogId) -> InspectRoomRepository {
    InspectRoomRepository {
        actor: PrivateRoomActor::Person(principal(who)),
        catalog_id,
    }
}

fn public_room_candidate(
    catalog_id: RoomCatalogId,
    owner: PrincipalId,
    code: &str,
) -> ProjectRoomCandidate {
    ProjectRoomCandidate {
        catalog_id,
        owner: Some(owner),
        active: true,
        sub_room: false,
        listed: true,
        share_code: Some(share_code(code)),
        held: false,
        bound: None,
    }
}

fn principal(principal_id: PrincipalId) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id,
        matrix_user_id: "@someone:matrix.test".to_owned(),
        display_name: "Someone".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: UtcMillis::new(i64::MAX).unwrap(),
        recently_authenticated: false,
    }
}

/// 这台电脑（或者网络 Agent 那边）的设备身份；令牌远没到期。
fn device(principal_id: PrincipalId) -> AuthenticatedDevice {
    AuthenticatedDevice {
        account: PrincipalAccount {
            principal: Principal::new(principal_id),
            matrix_user_id: "@someone:matrix.test".to_owned(),
            display_name: "Someone".to_owned(),
            avatar_content_id: None,
            locale: "zh-CN".to_owned(),
        },
        device_id: DeviceId::from_uuid(Uuid::from_u128(50)),
        access_token_expires_at: UtcMillis::new(i64::MAX).unwrap(),
    }
}

fn owner() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(1))
}

fn stranger() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(2))
}

fn room(sequence: u128) -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(100 + sequence))
}

fn created_room() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(999))
}

fn share_code(value: &str) -> RoomShareCode {
    RoomShareCode::parse(value).unwrap()
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}
