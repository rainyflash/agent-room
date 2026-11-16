//! 房主看、解绑、再核对一次这间房绑着的仓库（`specs/redesign/project-rooms.md` 的“网页上绑已有的房间”）。
//! 谁什么时候在哪个文件里核对上的只有房主看得到；房间里别的人、管理员都不行。
//!
//! 绑着哪个仓库（只有 `owner/repo`）谁登录了都能问（[`ProjectRoomService::room_repository_internal`]）：
//! 房间公开着时它本来就在 `/gh/<owner>/<repo>` 上，房间名旁边的勾要给房间里的每个人看。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    project_rooms::{ClaimCheckResult, GithubRepository},
    time::UtcMillis,
};

use crate::ports::{
    ProjectRoomCandidate, RepositoryBindingRecord, RepositoryReadFailure, UnbindRepositoryOutcome,
};

use super::{
    BindingCheckResult, BindingCheckView, BindingView, InspectRoomRepository,
    MANUAL_CHECK_INTERVAL_MILLIS, ProjectRoomFailure, ProjectRoomFailureKind, ProjectRoomResult,
    ProjectRoomService, after, repository_failure,
};

/// 记着的“再核对一次”最多留几间房，满了先扔掉过了一分钟的。
const MAX_REMEMBERED_BINDING_CHECKS: usize = 4_096;

impl ProjectRoomService {
    pub(super) async fn binding_internal(
        &self,
        request: InspectRoomRepository,
    ) -> ProjectRoomResult<Option<BindingView>> {
        const OPERATION: &str = "project_room.binding.inspect";
        let principal_id = self.claimant(&request.actor, OPERATION)?;
        let candidate = self
            .owned_room(principal_id, request.catalog_id, OPERATION)
            .await?;
        let binding = self
            .store
            .binding_for_room(request.catalog_id)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?;
        Ok(binding.map(|binding| BindingView {
            binding,
            share_code: candidate.share_code,
        }))
    }

    /// 公开着、没关、平台没拿掉、有分享码的主房间绑着的仓库，和 `/gh/` 页认房间的规矩一样。别的一律
    /// `None`：不说是没有这间房、没绑还是不公开了，免得拿它探私人房间在不在。
    pub(super) async fn room_repository_internal(
        &self,
        request: InspectRoomRepository,
    ) -> ProjectRoomResult<Option<GithubRepository>> {
        const OPERATION: &str = "project_room.binding.public";
        self.claimant(&request.actor, OPERATION)?;
        let candidate = self
            .store
            .project_room_candidate(request.catalog_id)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?;
        Ok(candidate.and_then(|room| {
            let public = room.active
                && room.listed
                && !room.held
                && !room.sub_room
                && room.share_code.is_some();
            if public { room.bound } else { None }
        }))
    }

    pub(super) async fn unbind_internal(
        &self,
        request: InspectRoomRepository,
    ) -> ProjectRoomResult<()> {
        const OPERATION: &str = "project_room.binding.unbind";
        let principal_id = self.claimant(&request.actor, OPERATION)?;
        self.owned_room(principal_id, request.catalog_id, OPERATION)
            .await?;
        let outcome = self
            .store
            .unbind_by_owner(request.catalog_id, principal_id)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?;
        match outcome {
            UnbindRepositoryOutcome::Unbound | UnbindRepositoryOutcome::NotBound => Ok(()),
            UnbindRepositoryOutcome::NotOwner => Err(ProjectRoomFailure::new(
                OPERATION,
                ProjectRoomFailureKind::Forbidden,
            )),
            UnbindRepositoryOutcome::LimitReached => Err(ProjectRoomFailure::new(
                OPERATION,
                ProjectRoomFailureKind::LimitReached,
            )),
        }
    }

    /// 再核对一次：那一行还在，就记下这次是谁、什么时候、在哪个文件里找到的；不在也不解绑（认上以后
    /// 不定期复查），只照实说。
    pub(super) async fn check_binding_internal(
        &self,
        request: InspectRoomRepository,
    ) -> ProjectRoomResult<BindingCheckView> {
        const OPERATION: &str = "project_room.binding.check";
        let principal_id = self.claimant(&request.actor, OPERATION)?;
        let candidate = self
            .owned_room(principal_id, request.catalog_id, OPERATION)
            .await?;
        let binding = self
            .store
            .binding_for_room(request.catalog_id)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
            .ok_or_else(|| ProjectRoomFailure::new(OPERATION, ProjectRoomFailureKind::NotFound))?;
        let share_code = candidate
            .share_code
            .clone()
            .ok_or_else(|| ProjectRoomFailure::new(OPERATION, ProjectRoomFailureKind::NotFound))?;
        let now = self.clock.now();
        self.admit_binding_check(request.catalog_id, now, OPERATION)?;
        let scanned = self
            .reader
            .scan(&binding.repository, std::slice::from_ref(&share_code))
            .await;
        let result = match scanned {
            Ok(scan) => match scan.file_of(&share_code) {
                Some(file) => BindingCheckResult::Found(file),
                None if scan.truncated => {
                    BindingCheckResult::Missing(ClaimCheckResult::FileTooLarge)
                }
                None => BindingCheckResult::Missing(ClaimCheckResult::NotInRepository),
            },
            Err(RepositoryReadFailure::Renamed) => {
                BindingCheckResult::Missing(ClaimCheckResult::RepositoryRenamed)
            }
            Err(RepositoryReadFailure::Unavailable { .. }) => {
                BindingCheckResult::Missing(ClaimCheckResult::GithubUnavailable)
            }
        };
        let binding = match result {
            BindingCheckResult::Found(file) => {
                let refreshed = RepositoryBindingRecord {
                    verified_by: principal_id,
                    verified_at: now,
                    verified_file: file,
                    ..binding
                };
                let still_bound = self
                    .store
                    .refresh_binding(&refreshed)
                    .await
                    .map_err(|error| repository_failure(OPERATION, &error))?;
                if !still_bound {
                    return Err(ProjectRoomFailure::new(
                        OPERATION,
                        ProjectRoomFailureKind::NotFound,
                    ));
                }
                refreshed
            }
            BindingCheckResult::Missing(_) => binding,
        };
        Ok(BindingCheckView {
            binding: BindingView {
                binding,
                share_code: Some(share_code),
            },
            checked_at: now,
            result,
        })
    }

    /// 这间房得是他的：没有这间房是没找到，不是房主是不让看。
    async fn owned_room(
        &self,
        principal_id: PrincipalId,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> ProjectRoomResult<ProjectRoomCandidate> {
        let candidate = self
            .store
            .project_room_candidate(catalog_id)
            .await
            .map_err(|error| repository_failure(operation, &error))?
            .ok_or_else(|| ProjectRoomFailure::new(operation, ProjectRoomFailureKind::NotFound))?;
        if candidate.owner == Some(principal_id) {
            Ok(candidate)
        } else {
            Err(ProjectRoomFailure::new(
                operation,
                ProjectRoomFailureKind::Forbidden,
            ))
        }
    }

    /// 每间房每分钟最多再核对一次。
    fn admit_binding_check(
        &self,
        catalog_id: RoomCatalogId,
        now: UtcMillis,
        operation: &'static str,
    ) -> ProjectRoomResult<()> {
        let mut checks = self
            .binding_checks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(last) = checks.get(&catalog_id) {
            let retry_at = after(*last, MANUAL_CHECK_INTERVAL_MILLIS);
            if now < retry_at {
                return Err(ProjectRoomFailure::new(
                    operation,
                    ProjectRoomFailureKind::CheckTooSoon,
                )
                .retrying_at(retry_at));
            }
        }
        if checks.len() >= MAX_REMEMBERED_BINDING_CHECKS {
            checks.retain(|_, last| now < after(*last, MANUAL_CHECK_INTERVAL_MILLIS));
        }
        checks.insert(catalog_id, now);
        Ok(())
    }
}
