//! 认领 GitHub 仓库、现在核对（3P-4a）：不要会话，原样转给控制面，拒绝换成 `bridge.project_room.*`。

use agent_room_bridge_core::project_rooms::{
    ControlPlaneProjectRoomGateway, ProjectRoomFailure, ProjectRoomFailureKind, ProjectRoomResult,
    RepositoryClaim, RepositoryClaimCheck, RepositoryClaimStatus,
};
use agent_room_bridge_ipc::{
    IpcCheckRepositoryClaimRequest, IpcClaimRepositoryRequest, IpcRepositoryClaimStatus,
};
use agent_room_domain::{
    ids::RepositoryClaimId, project_rooms::GithubRepository, room_listing::RoomShareCode,
    time::UtcMillis,
};

use super::*;

const CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const CLAIM_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e49";

/// 控制面的认领接口：记下每次调用；`failure` 设上时回答它。
#[derive(Default)]
pub(super) struct ProjectRoomsFake {
    calls: std::sync::Mutex<Vec<String>>,
    failure: std::sync::Mutex<Option<ProjectRoomFailureKind>>,
}

impl ProjectRoomsFake {
    fn answer(
        &self,
        call: String,
        status: RepositoryClaimStatus,
    ) -> ProjectRoomResult<RepositoryClaim> {
        self.calls.lock().unwrap().push(call);
        if let Some(kind) = *self.failure.lock().unwrap() {
            return Err(ProjectRoomFailure::new(kind));
        }
        let bound = status == RepositoryClaimStatus::Bound;
        Ok(RepositoryClaim {
            claim_id: (!bound)
                .then(|| RepositoryClaimId::from_uuid(Uuid::parse_str(CLAIM_ID).unwrap())),
            repository: GithubRepository::parse("octo/hello").unwrap(),
            share_code: RoomShareCode::parse("aB3xYz").unwrap(),
            share_url: "https://agent-room.test/r/aB3xYz".to_owned(),
            agents_line: "Agent Room: this project's room is https://agent-room.test/r/aB3xYz."
                .to_owned(),
            badge_markdown: "[![Agent Room](b)](https://agent-room.test/r/aB3xYz)".to_owned(),
            status,
            catalog_id: bound
                .then(|| RoomCatalogId::from_uuid(Uuid::parse_str(CATALOG_ID).unwrap())),
            claimed_at: (!bound).then(|| UtcMillis::new(1_000).unwrap()),
            last_check: (!bound).then(|| RepositoryClaimCheck {
                at: UtcMillis::new(2_000).unwrap(),
                result: "not_in_repository".to_owned(),
            }),
        })
    }
}

impl ControlPlaneProjectRoomGateway for ProjectRoomsFake {
    fn claim<'a>(
        &'a self,
        repository: &'a GithubRepository,
        target: Option<RoomCatalogId>,
    ) -> PortFuture<'a, ProjectRoomResult<RepositoryClaim>> {
        let call = format!(
            "claim {} {}",
            repository.full_name(),
            target.map_or_else(|| "-".to_owned(), |id| id.to_string())
        );
        let answer = self.answer(call, RepositoryClaimStatus::Pending);
        Box::pin(async move { answer })
    }

    fn check(
        &self,
        claim_id: RepositoryClaimId,
    ) -> PortFuture<'_, ProjectRoomResult<RepositoryClaim>> {
        let answer = self.answer(
            format!("check {}", claim_id.as_uuid()),
            RepositoryClaimStatus::Bound,
        );
        Box::pin(async move { answer })
    }
}

pub(super) fn access(gateway: Arc<ProjectRoomsFake>) -> ProjectRoomAccess {
    ProjectRoomAccess { gateway }
}

fn handler(gateway: Arc<ProjectRoomsFake>) -> SessionAwareIpcHandler {
    SessionAwareIpcHandler {
        default: Arc::new(RefusingHandler),
        sessions: Arc::new(HostSessionRegistry::new(Arc::new(TestFactory::default()))),
        connection_status: Arc::new(ReadyStatus),
        room_directory: Arc::new(DirectoryFake::default()),
        invitations: InvitationSlot::default(),
        join_codes: join_codes(Arc::new(JoinCodesFake::default()), Arc::default()),
        rooms: rooms::access(Arc::default(), Arc::default()),
        project_rooms: access(gateway),
    }
}

#[tokio::test]
async fn 认领和核对不要会话_原样转给控制面() {
    let gateway = Arc::new(ProjectRoomsFake::default());
    let handler = handler(gateway.clone());

    let IpcResponse::RepositoryClaim { claim } = handler
        .dispatch(IpcMethod::ClaimRepository(IpcClaimRepositoryRequest {
            repository: "Octo/Hello".to_owned(),
            catalog_id: Some(CATALOG_ID.to_owned()),
        }))
        .await
        .expect("认领了")
    else {
        panic!("应回认领");
    };
    assert_eq!(claim.status, IpcRepositoryClaimStatus::Pending);
    assert_eq!(claim.claim_id.as_deref(), Some(CLAIM_ID));
    assert_eq!(claim.share_code, "aB3xYz");
    assert_eq!(claim.claimed_at_unix_ms, Some(1_000));
    assert_eq!(
        claim.last_check.map(|check| check.result).as_deref(),
        Some("not_in_repository")
    );

    let IpcResponse::RepositoryClaim { claim } = handler
        .dispatch(IpcMethod::CheckRepositoryClaim(
            IpcCheckRepositoryClaimRequest {
                claim_id: CLAIM_ID.to_owned(),
            },
        ))
        .await
        .expect("核对了")
    else {
        panic!("应回认领");
    };
    assert_eq!(claim.status, IpcRepositoryClaimStatus::Bound);
    assert_eq!(claim.claim_id, None);
    assert_eq!(claim.catalog_id.as_deref(), Some(CATALOG_ID));

    assert_eq!(
        *gateway.calls.lock().unwrap(),
        [
            format!("claim Octo/Hello {CATALOG_ID}"),
            format!("check {CLAIM_ID}"),
        ]
    );
}

#[tokio::test]
async fn 控制面的拒绝换成认领自己的错误码() {
    for (kind, code, category, retryable) in [
        (
            ProjectRoomFailureKind::TooManyClaims,
            "bridge.project_room.too_many_claims",
            IpcErrorCategory::Conflict,
            false,
        ),
        (
            ProjectRoomFailureKind::CheckTooSoon,
            "bridge.project_room.check_too_soon",
            IpcErrorCategory::Conflict,
            false,
        ),
        (
            ProjectRoomFailureKind::RoomNotEligible,
            "bridge.project_room.room_not_eligible",
            IpcErrorCategory::Conflict,
            false,
        ),
        (
            ProjectRoomFailureKind::Forbidden,
            "bridge.project_room.forbidden",
            IpcErrorCategory::Authorization,
            false,
        ),
        (
            ProjectRoomFailureKind::NotAuthorized,
            "bridge.host_session.device_authorization_required",
            IpcErrorCategory::Authentication,
            false,
        ),
        (
            ProjectRoomFailureKind::Unsupported,
            "bridge.project_room.unsupported",
            IpcErrorCategory::IncompatibleVersion,
            false,
        ),
        (
            ProjectRoomFailureKind::ControlPlaneUnavailable,
            "bridge.project_room.unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
    ] {
        let gateway = Arc::new(ProjectRoomsFake::default());
        *gateway.failure.lock().unwrap() = Some(kind);
        let failure = handler(gateway)
            .dispatch(IpcMethod::ClaimRepository(IpcClaimRepositoryRequest {
                repository: "octo/hello".to_owned(),
                catalog_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(
            failure,
            BridgeIpcDispatchFailure::new(code, category, retryable),
            "{kind:?}"
        );
    }
}
