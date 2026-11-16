//! 核对认领：认领的人点“现在核对”，或者后台一轮一轮地来。
//!
//! 先在库里记下开始核对的这一刻（别处就不再同时核对它），再读 GitHub：
//! - 仓库已经绑在要绑的那间房上：认领删掉，回那间房；
//! - 仓库绑着别的房间：那间房被平台冻结、拿掉了，或者它的链接还在仓库里，就一直等；
//! - 找到了这个认领的那一行：带着已有房间的绑上它，没带的建一间项目房间再绑上，原来绑着别的房间的
//!   一起挪过来；
//! - 别的都记成这次核对的结果，下次再来。

use agent_room_domain::{
    ids::RoomCatalogId,
    project_rooms::{ClaimCheckResult, RepositoryFile},
    time::UtcMillis,
};

use crate::{
    persistence::RepositoryError,
    ports::{
        BindRepository, BindRepositoryOutcome, BoundRepositoryRoom, CreateProjectRoom,
        DueClaimQuery, ProjectRoomCreation, RepositoryCheckRecord, RepositoryClaimCheckStart,
        RepositoryClaimRecord, RepositoryReadFailure,
    },
};

use super::{
    CLAIM_LIFETIME_MILLIS, CLAIMS_PER_ROUND, ClaimRound, ClaimView, InspectClaim,
    MANUAL_CHECK_INTERVAL_MILLIS, OLD_CHECK_INTERVAL_MILLIS, ProjectRoomFailure,
    ProjectRoomFailureKind, ProjectRoomResult, ProjectRoomService, YOUNG_CHECK_INTERVAL_MILLIS,
    YOUNG_CLAIM_MILLIS, before, repository_failure,
};

/// 在仓库里找到了这个认领的那一行。
#[derive(Debug, Clone, Copy)]
struct Found {
    file: RepositoryFile,
    verified_at: UtcMillis,
    /// 原来绑着这个仓库的房间，绑定挪过来。
    replacing: Option<RoomCatalogId>,
}

/// 一次核对的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckOutcome {
    /// 认上了，绑在这间房上。
    Bound(RoomCatalogId),
    /// 还在等，这次的结果记下了。
    Pending(ClaimCheckResult),
    /// 要绑的那间房变了（不公开了、关了、换了房主或分享码），认领删了。
    Dropped,
    /// 认领已经没了（别处认上了、撤了）。
    Gone,
}

impl ProjectRoomService {
    pub(super) async fn check_claim_internal(
        &self,
        request: InspectClaim,
    ) -> ProjectRoomResult<ClaimView> {
        const OPERATION: &str = "project_room.claim.check";
        let principal_id = self.claimant(&request.actor, OPERATION)?;
        let record = self
            .own_claim(principal_id, request.claim_id, OPERATION)
            .await?;
        let now = self.clock.now();
        let started = self
            .store
            .start_check(record.id, now, before(now, MANUAL_CHECK_INTERVAL_MILLIS))
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?;
        let started = match started {
            RepositoryClaimCheckStart::Started(started) => started,
            RepositoryClaimCheckStart::TooSoon { retry_at } => {
                return Err(ProjectRoomFailure::new(
                    OPERATION,
                    ProjectRoomFailureKind::CheckTooSoon,
                )
                .retrying_at(retry_at));
            }
            RepositoryClaimCheckStart::Gone => {
                return Err(ProjectRoomFailure::new(
                    OPERATION,
                    ProjectRoomFailureKind::NotFound,
                ));
            }
        };
        let outcome = self
            .verify(&started, now)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?;
        match outcome {
            CheckOutcome::Bound(catalog_id) => Ok(ClaimView::bound(
                started.repository,
                started.share_code,
                catalog_id,
            )),
            CheckOutcome::Pending(result) => Ok(ClaimView::pending(RepositoryClaimRecord {
                last_check: Some(RepositoryCheckRecord { at: now, result }),
                ..started
            })),
            CheckOutcome::Dropped => Err(ProjectRoomFailure::new(
                OPERATION,
                ProjectRoomFailureKind::RoomNotEligible,
            )),
            CheckOutcome::Gone => Err(ProjectRoomFailure::new(
                OPERATION,
                ProjectRoomFailureKind::NotFound,
            )),
        }
    }

    /// 后台一轮：作废 7 天没认上的，再按先后核对该核对的。全站这一分钟的次数用完了、或者 GitHub 限了
    /// 次数正停着，剩下的下一轮再来。
    pub(super) async fn check_due_internal(&self) -> Result<ClaimRound, RepositoryError> {
        let now = self.clock.now();
        let mut round = ClaimRound {
            expired: self
                .store
                .expire_claims(before(now, CLAIM_LIFETIME_MILLIS))
                .await?,
            ..ClaimRound::default()
        };
        let due = self
            .store
            .due_claims(DueClaimQuery {
                now,
                young_since: before(now, YOUNG_CLAIM_MILLIS),
                young_checked_before: before(now, YOUNG_CHECK_INTERVAL_MILLIS),
                old_checked_before: before(now, OLD_CHECK_INTERVAL_MILLIS),
                limit: CLAIMS_PER_ROUND,
            })
            .await?;
        for claim_id in due {
            if self.reader.next_read_at().is_some() {
                round.deferred = true;
                break;
            }
            let now = self.clock.now();
            // 别的控制面这几分钟里刚核对过的跳过；后台最勤是 5 分钟一次。
            let started = self
                .store
                .start_check(claim_id, now, before(now, YOUNG_CHECK_INTERVAL_MILLIS))
                .await?;
            let RepositoryClaimCheckStart::Started(record) = started else {
                continue;
            };
            round.checked += 1;
            match self.verify(&record, now).await? {
                CheckOutcome::Bound(_) => round.bound += 1,
                CheckOutcome::Pending(_) => round.pending += 1,
                CheckOutcome::Dropped => round.dropped += 1,
                CheckOutcome::Gone => {}
            }
        }
        Ok(round)
    }

    /// 已经记下开始核对的一个认领：读 GitHub，认上了就绑上（或者建房再绑上），没认上记下为什么。
    async fn verify(
        &self,
        record: &RepositoryClaimRecord,
        started_at: UtcMillis,
    ) -> Result<CheckOutcome, RepositoryError> {
        let bound = self.store.bound_room(&record.repository).await?;
        if let Some(bound) = &bound {
            if Some(bound.binding.catalog_id) == record.target {
                // 已经绑在要绑的那间房上了（比如上次绑上以后删认领那一步没做成）。
                self.store
                    .delete_claim(record.id, record.principal_id)
                    .await?;
                return Ok(CheckOutcome::Bound(bound.binding.catalog_id));
            }
            if bound.held {
                return self
                    .pending(record, started_at, ClaimCheckResult::RepositoryTaken)
                    .await;
            }
        }
        let previous_code = previous_share_code(bound.as_ref(), record);
        let mut codes = vec![record.share_code.clone()];
        codes.extend(previous_code.iter().cloned());
        let scan = match self.reader.scan(&record.repository, &codes).await {
            Ok(scan) => scan,
            Err(RepositoryReadFailure::Renamed) => {
                return self
                    .pending(record, started_at, ClaimCheckResult::RepositoryRenamed)
                    .await;
            }
            Err(RepositoryReadFailure::Unavailable { .. }) => {
                return self
                    .pending(record, started_at, ClaimCheckResult::GithubUnavailable)
                    .await;
            }
        };
        let Some(file) = scan.file_of(&record.share_code) else {
            let result = if scan.truncated {
                ClaimCheckResult::FileTooLarge
            } else {
                ClaimCheckResult::NotInRepository
            };
            return self.pending(record, started_at, result).await;
        };
        // 原来那间房的链接还在仓库里：两行都在，新的认领一直等着。
        if previous_code
            .as_ref()
            .is_some_and(|code| scan.file_of(code).is_some())
        {
            return self
                .pending(record, started_at, ClaimCheckResult::RepositoryTaken)
                .await;
        }
        let replacing = bound.map(|bound| bound.binding.catalog_id);
        let found = Found {
            file,
            verified_at: self.clock.now(),
            replacing,
        };
        if let Some(catalog_id) = record.target {
            self.bind_target(record, catalog_id, found, started_at)
                .await
        } else {
            self.create_room(record, found, started_at).await
        }
    }

    /// 认上了带房间的认领：绑到那间房上。房间不能绑了，认领删掉。
    async fn bind_target(
        &self,
        record: &RepositoryClaimRecord,
        catalog_id: RoomCatalogId,
        found: Found,
        started_at: UtcMillis,
    ) -> Result<CheckOutcome, RepositoryError> {
        let outcome = self
            .store
            .bind_room(&BindRepository {
                claim_id: record.id,
                catalog_id,
                repository: record.repository.clone(),
                verified_by: record.principal_id,
                verified_at: found.verified_at,
                verified_file: found.file,
                share_code: record.share_code.clone(),
                replacing: found.replacing,
            })
            .await?;
        match outcome {
            BindRepositoryOutcome::Bound => Ok(CheckOutcome::Bound(catalog_id)),
            BindRepositoryOutcome::ClaimGone => Ok(CheckOutcome::Gone),
            BindRepositoryOutcome::RoomNotEligible => {
                self.store
                    .delete_claim(record.id, record.principal_id)
                    .await?;
                Ok(CheckOutcome::Dropped)
            }
            BindRepositoryOutcome::RepositoryTaken => {
                self.pending(record, started_at, ClaimCheckResult::RepositoryTaken)
                    .await
            }
        }
    }

    /// 认上了没带房间的认领：替认领的人建一间项目房间再绑上。没建成的认领留着，下次再试。
    async fn create_room(
        &self,
        record: &RepositoryClaimRecord,
        found: Found,
        started_at: UtcMillis,
    ) -> Result<CheckOutcome, RepositoryError> {
        let created = self
            .creator
            .create_project_room(CreateProjectRoom {
                claim_id: record.id,
                owner: record.principal_id,
                repository: record.repository.clone(),
                share_code: record.share_code.clone(),
                verified_at: found.verified_at,
                verified_file: found.file,
                replacing: found.replacing,
            })
            .await;
        match created {
            ProjectRoomCreation::Created(catalog_id) => Ok(CheckOutcome::Bound(catalog_id)),
            ProjectRoomCreation::ClaimGone => Ok(CheckOutcome::Gone),
            ProjectRoomCreation::RepositoryTaken => {
                self.pending(record, started_at, ClaimCheckResult::RepositoryTaken)
                    .await
            }
            ProjectRoomCreation::Failed => {
                self.pending(record, started_at, ClaimCheckResult::CreationFailed)
                    .await
            }
        }
    }

    async fn pending(
        &self,
        record: &RepositoryClaimRecord,
        started_at: UtcMillis,
        result: ClaimCheckResult,
    ) -> Result<CheckOutcome, RepositoryError> {
        self.store
            .finish_check(record.id, started_at, result)
            .await?;
        Ok(CheckOutcome::Pending(result))
    }
}

/// 原来绑着这个仓库的那间房现在的码。和这个认领的码一样时不另找（绑已有房间的认领就是那间房自己）。
fn previous_share_code(
    bound: Option<&BoundRepositoryRoom>,
    record: &RepositoryClaimRecord,
) -> Option<agent_room_domain::room_listing::RoomShareCode> {
    bound
        .and_then(|bound| bound.share_code.clone())
        .filter(|code| *code != record.share_code)
}
