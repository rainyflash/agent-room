//! 申请和审批：要申请的房间留一句话等批准；房主和管理员看得到、批准或拒绝，批准以后以房主的名义
//! 邀请；被拒的一天以内不能再申请；处理完的 30 天后删掉。

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    ports::JoinRequestRecord,
    private_rooms::{
        AnswerJoinRequest, InviteByLink, JoinPublicRoom, JoinRequestCleanupUseCases,
        JoinRequestState, PublicJoinFailureKind, PublicJoinMembership, PublicRoomJoinUseCases,
        RequestToJoin,
    },
};
use agent_room_domain::{
    ids::PrincipalId,
    private_rooms::{PrivateRoom, PrivateRoomPermissions},
    room_join_requests::{
        JOIN_REQUEST_REJECTION_COOLDOWN_MILLIS, JOIN_REQUEST_RETENTION_MILLIS, JoinRequestMessage,
        JoinRequestStatus,
    },
    room_listing::RoomJoinMode,
};

use super::{
    Fixture, NOW, catalog_id, listing, owner, person, principal, speak_permissions, speaker,
    stranger, time,
};

#[tokio::test]
async fn 要申请的房间_留一句话等批准_再点一次还是那一个() {
    let fixture = request_room();
    let state = fixture
        .service
        .request(ask(stranger(), "  想来听听\n"))
        .await
        .expect("记下申请");
    assert_eq!(
        state,
        JoinRequestState::Pending {
            requested_at: time(NOW)
        }
    );
    let again = fixture
        .service
        .request(ask(stranger(), "换一句"))
        .await
        .expect("还是那一个");
    assert_eq!(again, state);
    let records = fixture.requests.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].message.as_str(), "想来听听", "留言不改");
    assert_eq!(records[0].status, JoinRequestStatus::Pending);
    assert!(fixture.invitations.calls().is_empty(), "批准以前不邀请");

    let too_long = "长".repeat(201);
    for message in [too_long.as_str(), "带\r回车"] {
        let failure = fixture
            .service
            .request(ask(person(5), message))
            .await
            .unwrap_err();
        assert_eq!(failure.kind(), PublicJoinFailureKind::InvalidRequest);
    }
    assert_eq!(fixture.requests.records().len(), 1);
}

#[tokio::test]
async fn 直接进的房间申请就直接进_只能邀请的_封禁的_不公开的都不记() {
    let open = Fixture::new(managed_room(), Some(listing(true, RoomJoinMode::Open)));
    assert_eq!(
        open.service.request(ask(stranger(), "")).await.unwrap(),
        JoinRequestState::Member(PublicJoinMembership::Invited)
    );
    assert_eq!(open.invitations.calls().len(), 1);
    assert_eq!(
        open.service.request(ask(speaker(), "")).await.unwrap(),
        JoinRequestState::Member(PublicJoinMembership::Joined)
    );
    assert!(open.requests.records().is_empty());

    for (fixture, kind) in [
        (
            Fixture::new(
                managed_room(),
                Some(listing(true, RoomJoinMode::InviteOnly)),
            ),
            PublicJoinFailureKind::InviteOnly,
        ),
        (
            Fixture::new(
                banned_stranger_room(),
                Some(listing(true, RoomJoinMode::Request)),
            ),
            PublicJoinFailureKind::Banned,
        ),
        (
            Fixture::new(managed_room(), Some(listing(false, RoomJoinMode::Request))),
            PublicJoinFailureKind::NotFound,
        ),
    ] {
        let failure = fixture
            .service
            .request(ask(stranger(), ""))
            .await
            .unwrap_err();
        assert_eq!(failure.kind(), kind);
        assert!(fixture.requests.records().is_empty());
        assert!(fixture.invitations.calls().is_empty());
    }
}

#[tokio::test]
async fn 房主和管理员看得到在等的申请_批准以后以房主的名义邀请() {
    let fixture = request_room();
    fixture
        .service
        .request(ask(stranger(), "想来听听"))
        .await
        .unwrap();
    for (actor, kind) in [
        (speaker(), PublicJoinFailureKind::Forbidden),
        (person(9), PublicJoinFailureKind::NotFound),
    ] {
        let failure = fixture.service.pending_requests(look(actor)).await;
        assert_eq!(failure.unwrap_err().kind(), kind);
        let failure = fixture.service.answer(answer(actor, true)).await;
        assert_eq!(failure.unwrap_err().kind(), kind);
    }
    for actor in [owner(), manager()] {
        let pending = fixture
            .service
            .pending_requests(look(actor))
            .await
            .expect("看得到");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].record.requester, stranger());
        assert_eq!(pending[0].record.message.as_str(), "想来听听");
        assert_eq!(
            pending[0].requester_display_name, "xiaolin",
            "像邮箱的只留 @ 前面"
        );
    }

    assert_eq!(
        fixture
            .service
            .answer(answer(manager(), true))
            .await
            .unwrap(),
        JoinRequestStatus::Approved
    );
    assert_eq!(
        fixture.invitations.calls(),
        [InviteByLink {
            catalog_id: catalog_id(),
            link_creator: owner(),
            target_principal_id: stranger(),
            permissions: speak_permissions(),
        }]
    );
    let record = fixture.requests.records().remove(0);
    assert_eq!(record.status, JoinRequestStatus::Approved);
    assert_eq!(record.decided_by, Some(manager()));
    assert_eq!(record.decided_at, Some(time(NOW)));

    // 处理过了，再点一次说没有这个申请。
    let failure = fixture.service.answer(answer(owner(), false)).await;
    assert_eq!(failure.unwrap_err().kind(), PublicJoinFailureKind::NotFound);
    let pending = fixture.service.pending_requests(look(owner())).await;
    assert!(pending.unwrap().is_empty());
}

#[tokio::test]
async fn 被拒的一天以内不能再申请_过了一天可以_撤回的马上可以() {
    let fixture = request_room();
    fixture.service.request(ask(stranger(), "")).await.unwrap();
    assert_eq!(
        fixture
            .service
            .answer(answer(owner(), false))
            .await
            .unwrap(),
        JoinRequestStatus::Rejected
    );
    assert!(fixture.invitations.calls().is_empty());
    let failure = fixture
        .service
        .request(ask(stranger(), ""))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), PublicJoinFailureKind::RecentlyRejected);
    assert_eq!(
        failure.retry_at(),
        Some(time(NOW + JOIN_REQUEST_REJECTION_COOLDOWN_MILLIS))
    );

    let later = request_room();
    later.requests.seed(JoinRequestRecord {
        status: JoinRequestStatus::Rejected,
        decided_at: Some(time(NOW - JOIN_REQUEST_REJECTION_COOLDOWN_MILLIS)),
        decided_by: Some(owner()),
        ..waiting(stranger())
    });
    let state = later.service.request(ask(stranger(), "")).await;
    assert!(
        matches!(state, Ok(JoinRequestState::Pending { .. })),
        "过了一天"
    );

    let withdrawn = request_room();
    withdrawn
        .service
        .request(ask(stranger(), ""))
        .await
        .unwrap();
    withdrawn
        .service
        .withdraw(look(stranger()))
        .await
        .expect("撤回");
    withdrawn
        .service
        .withdraw(look(stranger()))
        .await
        .expect("没有在等的也算撤回了");
    let record = withdrawn.requests.records().remove(0);
    assert_eq!(record.status, JoinRequestStatus::Withdrawn);
    assert_eq!(record.decided_by, None);
    withdrawn
        .service
        .request(ask(stranger(), ""))
        .await
        .expect("撤回的马上可以再申请");
    assert_eq!(withdrawn.requests.records().len(), 2);
}

#[tokio::test]
async fn 批准时房间刚改回私密算刚变了_封禁的要先解封_拒绝照样能拒() {
    let unlisted = Fixture::new(managed_room(), Some(listing(false, RoomJoinMode::Request)));
    unlisted.requests.seed(waiting(stranger()));
    let failure = unlisted.service.answer(answer(owner(), true)).await;
    assert_eq!(failure.unwrap_err().kind(), PublicJoinFailureKind::Conflict);
    assert_eq!(
        unlisted.requests.records()[0].status,
        JoinRequestStatus::Pending,
        "没批准成就还在等"
    );
    assert_eq!(
        unlisted
            .service
            .answer(answer(owner(), false))
            .await
            .unwrap(),
        JoinRequestStatus::Rejected
    );

    let banned = Fixture::new(
        banned_stranger_room(),
        Some(listing(true, RoomJoinMode::Request)),
    );
    banned.requests.seed(waiting(stranger()));
    let failure = banned.service.answer(answer(owner(), true)).await;
    assert_eq!(failure.unwrap_err().kind(), PublicJoinFailureKind::Banned);

    // 申请等着的时候已经被邀请进来了：批准只记下，不再邀请一次。
    let mut invited_room = managed_room();
    invited_room
        .invite(owner(), stranger(), speak_permissions())
        .unwrap();
    let invited = Fixture::new(invited_room, Some(listing(true, RoomJoinMode::Request)));
    invited.requests.seed(waiting(stranger()));
    assert_eq!(
        invited.service.answer(answer(owner(), true)).await.unwrap(),
        JoinRequestStatus::Approved
    );
    for fixture in [&unlisted, &banned, &invited] {
        assert!(fixture.invitations.calls().is_empty());
    }
}

#[tokio::test]
async fn 平台拿掉的房间申请不了_在等的批不了_拒绝照样能拒() {
    let fixture = request_room();
    fixture.holds.hold();
    let failure = fixture
        .service
        .request(ask(person(5), "想来"))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), PublicJoinFailureKind::NotFound);

    fixture.requests.seed(waiting(stranger()));
    let failure = fixture.service.answer(answer(owner(), true)).await;
    assert_eq!(failure.unwrap_err().kind(), PublicJoinFailureKind::Conflict);
    assert_eq!(
        fixture
            .service
            .answer(answer(owner(), false))
            .await
            .unwrap(),
        JoinRequestStatus::Rejected
    );
    assert!(fixture.invitations.calls().is_empty());
    assert_eq!(fixture.requests.records().len(), 1, "没记新的申请");
}

#[tokio::test]
async fn 登录过期的申请_撤回_查看_处理都不行() {
    let fixture = request_room();
    fixture.requests.seed(waiting(stranger()));
    let expired = |principal_id| {
        let mut actor = principal(principal_id);
        actor.expires_at = time(NOW);
        actor
    };
    let failures = [
        fixture
            .service
            .request(RequestToJoin {
                actor: expired(person(5)),
                ..ask(person(5), "")
            })
            .await
            .map(|_| ()),
        fixture.service.withdraw(look_as(expired(stranger()))).await,
        fixture
            .service
            .pending_requests(look_as(expired(owner())))
            .await
            .map(|_| ()),
        fixture
            .service
            .answer(AnswerJoinRequest {
                actor: expired(owner()),
                ..answer(owner(), true)
            })
            .await
            .map(|_| ()),
    ];
    for failure in failures {
        assert_eq!(
            failure.unwrap_err().kind(),
            PublicJoinFailureKind::Forbidden
        );
    }
    assert_eq!(fixture.requests.records(), [waiting(stranger())]);
}

#[tokio::test]
async fn 定时清理_处理完三十天的删掉() {
    let fixture = request_room();
    fixture.service.sweep().await.expect("清理");
    assert_eq!(
        *fixture.requests.sweeps.lock().unwrap(),
        [(time(NOW), time(NOW - JOIN_REQUEST_RETENTION_MILLIS))]
    );
}

fn request_room() -> Fixture {
    Fixture::new(managed_room(), Some(listing(true, RoomJoinMode::Request)))
}

/// 房主、一个管理员和一个只能发言的成员。
fn managed_room() -> PrivateRoom {
    let mut room = PrivateRoom::create(catalog_id(), owner());
    for (member, permissions) in [
        (manager(), PrivateRoomPermissions::ALL),
        (speaker(), speak_permissions()),
    ] {
        room.invite(owner(), member, permissions).unwrap();
        room.accept_invitation(member).unwrap();
    }
    room
}

fn banned_stranger_room() -> PrivateRoom {
    let mut room = managed_room();
    room.invite(owner(), stranger(), speak_permissions())
        .unwrap();
    room.ban_member(owner(), stranger()).unwrap();
    room
}

fn waiting(requester: PrincipalId) -> JoinRequestRecord {
    JoinRequestRecord {
        catalog_id: catalog_id(),
        requester,
        message: JoinRequestMessage::default(),
        status: JoinRequestStatus::Pending,
        requested_at: time(NOW - 5_000),
        decided_at: None,
        decided_by: None,
    }
}

fn ask(actor: PrincipalId, message: &str) -> RequestToJoin {
    RequestToJoin {
        actor: principal(actor),
        catalog_id: catalog_id(),
        message: message.to_owned(),
    }
}

fn look(actor: PrincipalId) -> JoinPublicRoom {
    look_as(principal(actor))
}

fn look_as(actor: AuthenticatedPrincipal) -> JoinPublicRoom {
    JoinPublicRoom {
        actor,
        catalog_id: catalog_id(),
    }
}

/// 处理陌生人的申请。
fn answer(actor: PrincipalId, approve: bool) -> AnswerJoinRequest {
    AnswerJoinRequest {
        actor: principal(actor),
        catalog_id: catalog_id(),
        requester: stranger(),
        approve,
    }
}

fn manager() -> PrincipalId {
    person(2)
}
