//! 发现页（`specs/public-rooms/design.md`）：不登录也看得到公开房间的目录和一间房的详情；登录的人
//! 另带上自己在每间房的处境（已加入、受邀、在等批准、被封禁），网页按它给按钮。
//!
//! 昵称里像邮箱的只留 @ 前面那段，和房间里的名字一样；按房主找时也只认这一段，免得拿邮箱的后半段
//! 搜出谁用的是哪家邮箱。
//!
//! 详情另带“现在几个在线”，见 [`online`]。

mod online;

use std::{collections::BTreeMap, sync::Arc};

use agent_room_domain::{
    ids::RoomCatalogId,
    room_listing::{RoomJoinMode, RoomShareCode},
    time::UtcMillis,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, PortFuture, PublicRoomDetailRecord, PublicRoomDirectoryStore,
        PublicRoomPresenceMatrix, PublicRoomRecord, PublicRoomSearch, PublicRoomSort,
        PublicRoomViewerState,
    },
    room_people::shown_display_name,
};

use online::OnlineCounter;
pub use online::{MAX_PRESENCE_CHECKS, ONLINE_COUNT_TTL_MILLIS, PublicRoomOnline};

/// 一页最多几间。
pub const PUBLIC_ROOM_PAGE_SIZE: u32 = 24;

/// 最多往后翻到第几间：目录是给人翻的，翻得再深的用搜索。
pub const MAX_PUBLIC_ROOM_OFFSET: u32 = 40 * PUBLIC_ROOM_PAGE_SIZE;

/// 搜索框里最多几个字。
pub const MAX_PUBLIC_ROOM_SEARCH_CHARS: usize = 100;

/// “一小时内有人说话”。
pub const RECENTLY_ACTIVE_MILLIS: i64 = 60 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicDirectoryFailureKind {
    /// 搜索的字太长、带着控制字符，或者翻得太深。
    InvalidRequest,
    /// 没有这个分享码，或者那间房不公开了、关了。
    NotFound,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicDirectoryFailure {
    operation: &'static str,
    kind: PublicDirectoryFailureKind,
}

impl PublicDirectoryFailure {
    pub const fn new(operation: &'static str, kind: PublicDirectoryFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> PublicDirectoryFailureKind {
        self.kind
    }
}

pub type PublicDirectoryResult<T> = Result<T, PublicDirectoryFailure>;

/// 翻目录。`viewer` 是登录了的人，没登录（或者登录过期了）的是 `None`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowsePublicRooms {
    pub viewer: Option<AuthenticatedPrincipal>,
    pub text: String,
    pub join_mode: Option<RoomJoinMode>,
    pub recently_active: bool,
    pub unencrypted_only: bool,
    /// 只要没有账号的网络 Agent 拿房间号进得去的：房主允许网络 Agent，也不是只能邀请的。
    pub network_agents_only: bool,
    pub sort: PublicRoomSort,
    pub offset: u32,
}

/// 目录里给人看的一间房：房主的昵称处理过了；登录的人带着自己的处境。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRoomView {
    pub room: PublicRoomRecord,
    pub viewer: Option<PublicRoomViewerState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRoomPage {
    pub rooms: Vec<PublicRoomView>,
    /// 还有下一页时，下一页从第几间开始。
    pub next_offset: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRoomDetailView {
    pub room: PublicRoomView,
    pub admin_display_names: Vec<String>,
    /// 问不到时是 `None`。
    pub online: Option<PublicRoomOnline>,
}

pub trait PublicRoomDirectoryUseCases: Send + Sync {
    fn browse(
        &self,
        request: BrowsePublicRooms,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomPage>>;

    /// 分享码对应的房间。不公开、关了、没有这个码、码的格式不对都回答没有。
    fn detail(
        &self,
        viewer: Option<AuthenticatedPrincipal>,
        share_code: String,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomDetailView>>;
}

pub struct PublicRoomDirectoryDependencies {
    pub directory: Arc<dyn PublicRoomDirectoryStore>,
    pub presence: Arc<dyn PublicRoomPresenceMatrix>,
    pub clock: Arc<dyn Clock>,
}

pub struct PublicRoomDirectoryService {
    directory: Arc<dyn PublicRoomDirectoryStore>,
    online: OnlineCounter,
    clock: Arc<dyn Clock>,
}

impl PublicRoomDirectoryService {
    pub fn new(dependencies: PublicRoomDirectoryDependencies) -> Self {
        Self {
            online: OnlineCounter::new(dependencies.directory.clone(), dependencies.presence),
            directory: dependencies.directory,
            clock: dependencies.clock,
        }
    }

    async fn browse_internal(
        &self,
        request: BrowsePublicRooms,
    ) -> PublicDirectoryResult<PublicRoomPage> {
        const OPERATION: &str = "public_room.browse";
        let invalid =
            || PublicDirectoryFailure::new(OPERATION, PublicDirectoryFailureKind::InvalidRequest);
        if !acceptable_search(&request.text) {
            return Err(invalid());
        }
        let text = request.text.trim();
        let text = (!text.is_empty()).then(|| text.to_owned());
        if request.offset > MAX_PUBLIC_ROOM_OFFSET {
            return Err(invalid());
        }
        let now = self.clock.now();
        let active_since = if request.recently_active {
            Some(
                UtcMillis::new(now.value().saturating_sub(RECENTLY_ACTIVE_MILLIS)).map_err(
                    |_| {
                        PublicDirectoryFailure::new(OPERATION, PublicDirectoryFailureKind::Internal)
                    },
                )?,
            )
        } else {
            None
        };
        let search = PublicRoomSearch {
            text,
            join_mode: request.join_mode,
            active_since,
            unencrypted_only: request.unencrypted_only,
            network_agents_only: request.network_agents_only,
            sort: request.sort,
            offset: request.offset,
            // 多拿一间，看还有没有下一页。
            limit: PUBLIC_ROOM_PAGE_SIZE + 1,
        };
        let mut rooms = self
            .directory
            .search_public_rooms(&search)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        let next_offset = (rooms.len() > PUBLIC_ROOM_PAGE_SIZE as usize)
            .then(|| request.offset + PUBLIC_ROOM_PAGE_SIZE)
            .filter(|offset| *offset <= MAX_PUBLIC_ROOM_OFFSET);
        rooms.truncate(PUBLIC_ROOM_PAGE_SIZE as usize);
        let rooms = self
            .with_viewer(request.viewer.as_ref(), rooms, now, OPERATION)
            .await?;
        Ok(PublicRoomPage { rooms, next_offset })
    }

    async fn detail_internal(
        &self,
        viewer: Option<AuthenticatedPrincipal>,
        share_code: String,
    ) -> PublicDirectoryResult<PublicRoomDetailView> {
        const OPERATION: &str = "public_room.detail";
        let not_found =
            || PublicDirectoryFailure::new(OPERATION, PublicDirectoryFailureKind::NotFound);
        let share_code = RoomShareCode::parse(&share_code).map_err(|_| not_found())?;
        let PublicRoomDetailRecord {
            room,
            admin_display_names,
        } = self
            .directory
            .public_room_by_share_code(&share_code)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .ok_or_else(not_found)?;
        let now = self.clock.now();
        let online = self.online.count(room.catalog_id, now).await;
        let room = self
            .with_viewer(viewer.as_ref(), vec![room], now, OPERATION)
            .await?
            .pop()
            .ok_or_else(not_found)?;
        Ok(PublicRoomDetailView {
            room,
            admin_display_names: admin_display_names
                .iter()
                .map(|name| shown_display_name(name))
                .collect(),
            online,
        })
    }

    /// 处理房主的昵称；登录着的人再查一次自己在这几间房的处境。
    async fn with_viewer(
        &self,
        viewer: Option<&AuthenticatedPrincipal>,
        rooms: Vec<PublicRoomRecord>,
        now: UtcMillis,
        operation: &'static str,
    ) -> PublicDirectoryResult<Vec<PublicRoomView>> {
        let viewer = viewer.filter(|viewer| now < viewer.expires_at);
        let states: BTreeMap<RoomCatalogId, PublicRoomViewerState> = match viewer {
            Some(viewer) if !rooms.is_empty() => {
                let ids: Vec<RoomCatalogId> = rooms.iter().map(|room| room.catalog_id).collect();
                self.directory
                    .public_room_viewer_states(viewer.principal_id, &ids)
                    .await
                    .map_err(|error| repository(operation, &error))?
                    .into_iter()
                    .collect()
            }
            _ => BTreeMap::new(),
        };
        Ok(rooms
            .into_iter()
            .map(|room| PublicRoomView {
                viewer: states.get(&room.catalog_id).copied(),
                room: PublicRoomRecord {
                    owner_display_name: shown_display_name(&room.owner_display_name),
                    ..room
                },
            })
            .collect())
    }
}

impl PublicRoomDirectoryUseCases for PublicRoomDirectoryService {
    fn browse(
        &self,
        request: BrowsePublicRooms,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomPage>> {
        Box::pin(self.browse_internal(request))
    }

    fn detail(
        &self,
        viewer: Option<AuthenticatedPrincipal>,
        share_code: String,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomDetailView>> {
        Box::pin(self.detail_internal(viewer, share_code))
    }
}

/// 搜索的字去掉首尾空白以后不超过 100 个字、不带控制字符；空的就是不搜。
fn acceptable_search(input: &str) -> bool {
    let text = input.trim();
    text.chars().count() <= MAX_PUBLIC_ROOM_SEARCH_CHARS && !text.chars().any(char::is_control)
}

const fn repository(operation: &'static str, error: &RepositoryError) -> PublicDirectoryFailure {
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
