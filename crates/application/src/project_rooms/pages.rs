//! 不登录也打得开的仓库地址（`specs/redesign/project-rooms.md` 的“猜得出来的地址”“README 徽章”，3P-3）：
//! `/gh/<owner>/<repo>` 和它的徽章只认绑着这个仓库、此刻公开着的房间，出和分享页同一份详情（带“现在
//! 几个在线”）。
//!
//! 这里不读 GitHub：地址谁都能编，不能让访客借它叫我们去敲 GitHub。没绑、认领了还没核对上、绑着的房间
//! 不公开了、关了、被平台拿掉，一律回答没有（[`PublicDirectoryFailureKind::NotFound`]），不分是哪一种。
//!
//! 分享页的预览图（3P-8a）另问一间房绑着哪个仓库（[`RepositoryRoomPages::bound_repository`]），图上写
//! `owner/repo` 和那个勾。

use std::sync::Arc;

use agent_room_domain::{ids::RoomCatalogId, project_rooms::GithubRepository};

use crate::{
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{BoundRepositoryRoom, PortFuture, ProjectRoomStore},
    public_room_directory::{
        PublicDirectoryFailure, PublicDirectoryFailureKind, PublicDirectoryResult,
        PublicRoomDetailView, PublicRoomDirectoryUseCases,
    },
};

pub trait RepositoryRoomPages: Send + Sync {
    /// 绑着这个仓库（不论大小写）、此刻公开着、没关、平台没拿掉的那间房，和分享页同一份详情。
    fn public_room(
        &self,
        repository: GithubRepository,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomDetailView>>;

    /// 这间房绑着的仓库，没绑是 `None`。不查房间公不公开：问之前已经由目录查过了。
    fn bound_repository(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, PublicDirectoryResult<Option<GithubRepository>>>;
}

pub struct RepositoryRoomPageService {
    store: Arc<dyn ProjectRoomStore>,
    directory: Arc<dyn PublicRoomDirectoryUseCases>,
}

impl RepositoryRoomPageService {
    pub fn new(
        store: Arc<dyn ProjectRoomStore>,
        directory: Arc<dyn PublicRoomDirectoryUseCases>,
    ) -> Self {
        Self { store, directory }
    }

    async fn public_room_internal(
        &self,
        repository: GithubRepository,
    ) -> PublicDirectoryResult<PublicRoomDetailView> {
        const OPERATION: &str = "project_room.public_page";
        let not_found =
            || PublicDirectoryFailure::new(OPERATION, PublicDirectoryFailureKind::NotFound);
        let bound = self
            .store
            .bound_room(&repository)
            .await
            .map_err(|error| store_failure(OPERATION, &error))?;
        // 平台冻结、拿掉的房间照旧占着仓库，可它不公开给人看；没有分享码的房间不公开着。
        let Some(BoundRepositoryRoom {
            binding,
            share_code: Some(share_code),
            held: false,
        }) = bound
        else {
            return Err(not_found());
        };
        // 公开着、没关、平台没拿掉由目录再查一遍，和分享页同一套。
        let detail = self
            .directory
            .detail(None, share_code.as_str().to_owned())
            .await?;
        // 两次读之间码换给了别的房间：只认绑着仓库的那一间。
        if detail.room.room.catalog_id != binding.catalog_id {
            return Err(not_found());
        }
        Ok(detail)
    }
}

impl RepositoryRoomPages for RepositoryRoomPageService {
    fn public_room(
        &self,
        repository: GithubRepository,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomDetailView>> {
        Box::pin(self.public_room_internal(repository))
    }

    fn bound_repository(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, PublicDirectoryResult<Option<GithubRepository>>> {
        Box::pin(async move {
            self.store
                .binding_for_room(catalog_id)
                .await
                .map(|binding| binding.map(|binding| binding.repository))
                .map_err(|error| store_failure("project_room.bound_repository", &error))
        })
    }
}

const fn store_failure(operation: &'static str, error: &RepositoryError) -> PublicDirectoryFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Unavailable => PublicDirectoryFailureKind::DependencyUnavailable,
        RepositoryErrorKind::Forbidden
        | RepositoryErrorKind::NotFound
        | RepositoryErrorKind::Conflict
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::CorruptData => PublicDirectoryFailureKind::Internal,
    };
    PublicDirectoryFailure::new(operation, kind)
}
