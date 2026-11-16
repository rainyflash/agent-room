//! 认上仓库以后替认领的人建项目房间（`specs/redesign/project-rooms.md` 的“一步建好并绑上”，3P-2b）。
//!
//! 和新建时就公开的房间走同一套：Matrix 上照旧隐藏、邀请制，不写 `m.room.encryption`，先写下公开的
//! 时间（`room.listing.v1`）。不同的几样：
//! - 由后台核对或者认领的人点“现在核对”触发，房主是认领的人，不看谁的登录；
//! - 名字是 `owner/repo`（超过 128 个字截断），简介空着；公开、直接进、让没有账号的网络 Agent 进；
//! - 分享码是认领的码：仓库里那一行写着它；
//! - 房间、公开设置、绑定和删认领在库里同一个事务，绑着仓库的公开房间不占名额，不查限额。

use agent_room_domain::{
    room_listing::{RoomJoinMode, RoomListingSettings},
    version::AggregateVersion,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    ports::{
        BindRepository, CreateProjectRoom, PortFuture, PrivateRoomListingRecord,
        ProjectRoomCreation, ProjectRoomCreator, ProjectRoomSaveOutcome,
    },
};

use super::{
    super::{
        CreatePrivateRoom, PrivateRoomActor, PrivateRoomFailureStage, PrivateRoomResult,
        failure::{domain, matrix, repository},
    },
    PrivateRoomService,
};

/// 房间名最多几个字（`RoomCatalog` 的上限）。`owner/repo` 最长 140 个。
const MAXIMUM_PROJECT_ROOM_NAME_CHARACTERS: usize = 128;

impl PrivateRoomService {
    async fn create_project_room_internal(
        &self,
        request: CreateProjectRoom,
    ) -> PrivateRoomResult<ProjectRoomCreation> {
        const OPERATION: &str = "private_room.create_project_room";
        let owner_matrix_user = self.principal_matrix_user(request.owner, OPERATION).await?;
        let now = self.clock.now();
        let settings = RoomListingSettings::new(
            true,
            RoomJoinMode::Open,
            RoomListingSettings::DEFAULT_MEMBER_PERMISSIONS,
            true,
        )
        .map_err(|error| domain(OPERATION, &error))?;
        // 建房那一套按“谁在建”取房主；这里没有登录，照认领的人造一个只在这一次里用的操作者。
        let creation = CreatePrivateRoom {
            actor: PrivateRoomActor::Person(AuthenticatedPrincipal {
                principal_id: request.owner,
                matrix_user_id: owner_matrix_user.as_str().to_owned(),
                display_name: String::new(),
                locale: String::new(),
                authenticated_at: now,
                expires_at: now,
                recently_authenticated: false,
            }),
            catalog_id: self.identifiers.room_catalog_id(),
            name: request
                .repository
                .full_name()
                .chars()
                .take(MAXIMUM_PROJECT_ROOM_NAME_CHARACTERS)
                .collect(),
            description: String::new(),
            retention_days: None,
            invitations: Vec::new(),
            created_by_agent: None,
            listing: Some(settings),
        };
        let prepared = self
            .prepare_creation(&creation, Some(owner_matrix_user), OPERATION)
            .await?;
        let matrix_room_id = self
            .matrix_provisioner
            .create(&prepared.matrix)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.matrix
            .set_speaking_batch(&matrix_room_id, &prepared.speaking_assignments)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        let snapshot = self.build_snapshot(
            creation.catalog_id,
            prepared.catalog,
            prepared.room,
            &matrix_room_id,
            OPERATION,
        )?;
        self.listing_matrix
            .mark_listed(&matrix_room_id, now)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        let listing = PrivateRoomListingRecord {
            catalog_id: creation.catalog_id,
            settings,
            share_code: request.share_code.clone(),
            first_listed_at: now,
            updated_by: request.owner,
            updated_at: now,
            version: AggregateVersion::INITIAL
                .next()
                .map_err(|error| domain(OPERATION, &error))?,
        };
        let binding = BindRepository {
            claim_id: request.claim_id,
            catalog_id: creation.catalog_id,
            repository: request.repository,
            verified_by: request.owner,
            verified_at: request.verified_at,
            verified_file: request.verified_file,
            share_code: request.share_code,
            replacing: request.replacing,
        };
        let saved = self
            .listings
            .create_project_room(&snapshot, &listing, &binding, now)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        // 没存成的几样，Matrix 上建好的房间就空在那里：没人在里面，也不在目录里。
        Ok(match saved {
            ProjectRoomSaveOutcome::Saved => ProjectRoomCreation::Created(creation.catalog_id),
            ProjectRoomSaveOutcome::ClaimGone => ProjectRoomCreation::ClaimGone,
            ProjectRoomSaveOutcome::RepositoryTaken => ProjectRoomCreation::RepositoryTaken,
            ProjectRoomSaveOutcome::ShareCodeTaken | ProjectRoomSaveOutcome::Stale => {
                ProjectRoomCreation::Failed
            }
        })
    }
}

impl ProjectRoomCreator for PrivateRoomService {
    fn create_project_room(
        &self,
        request: CreateProjectRoom,
    ) -> PortFuture<'_, ProjectRoomCreation> {
        Box::pin(async move {
            self.create_project_room_internal(request)
                .await
                .unwrap_or(ProjectRoomCreation::Failed)
        })
    }
}
