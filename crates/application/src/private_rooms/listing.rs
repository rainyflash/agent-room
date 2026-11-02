//! 公开房间的设置（`specs/public-rooms/design.md`）：房主切换公开和私密、定加入方式、新成员给什么
//! 权限、让不让没有账号的网络 Agent 进。房间里已加入的人都看得到设置和分享码（公开房间的成员
//! 都能分享），只有房主能改。
//!
//! 第一次公开时先在 Matrix 上写 `room.listing.v1`，再存设置：之后进来的人别人不给他们重发公开
//! 以前的房间钥匙。不加密的房间改回私密时先在 Matrix 上打开加密，再把目录上的加密标记改成 true；
//! 写不进 Matrix 就什么都不改，房主可以再点。

use std::sync::Arc;

use agent_room_domain::{
    DomainError,
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoomMember,
    room_listing::{RoomListingSettings, RoomShareCode},
    time::UtcMillis,
    version::AggregateVersion,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixRoomId, PortFuture,
        PrivateRoomListingMatrix, PrivateRoomListingRecord, PrivateRoomListingSaveOutcome,
        PrivateRoomListingStore, PrivateRoomSnapshot, PrivateRoomStore, RoomContentPolicyLookup,
        SecretFactory,
    },
};

/// 新生成的分享码撞上别的房间时最多再换几次。约 568 亿种里撞上一次已经很难，连撞几次多半是
/// 随机数坏了。
const SHARE_CODE_ATTEMPTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListingFailureKind {
    InvalidRequest,
    /// 登录过期，或者不是房主。
    Forbidden,
    /// 没有这个房间，或者他不在房间里。
    NotFound,
    /// 房主名下已经有一间公开房间。
    LimitReached,
    /// 别人刚改过，或者房间刚归档。
    Conflict,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListingFailure {
    operation: &'static str,
    kind: ListingFailureKind,
}

impl ListingFailure {
    pub const fn new(operation: &'static str, kind: ListingFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> ListingFailureKind {
        self.kind
    }
}

pub type ListingResult<T> = Result<T, ListingFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectListing {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateListing {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub settings: RoomListingSettings,
}

/// 房间此刻的公开设置。从没公开过的房间是 [`RoomListingSettings::UNLISTED`]，没有分享码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingView {
    pub settings: RoomListingSettings,
    pub share_code: Option<RoomShareCode>,
    pub first_listed_at: Option<UtcMillis>,
    /// 房间开没开端到端加密。新建时选了公开的不加密，改回私密以后加密。
    pub end_to_end_encrypted: bool,
}

pub trait PrivateRoomListingUseCases: Send + Sync {
    fn inspect(&self, request: InspectListing) -> PortFuture<'_, ListingResult<ListingView>>;

    fn update(&self, request: UpdateListing) -> PortFuture<'_, ListingResult<ListingView>>;
}

pub struct PrivateRoomListingDependencies {
    pub rooms: Arc<dyn PrivateRoomStore>,
    pub listings: Arc<dyn PrivateRoomListingStore>,
    pub policies: Arc<dyn RoomContentPolicyLookup>,
    pub matrix: Arc<dyn PrivateRoomListingMatrix>,
    pub secrets: Arc<dyn SecretFactory>,
    pub clock: Arc<dyn Clock>,
}

pub struct PrivateRoomListingService {
    rooms: Arc<dyn PrivateRoomStore>,
    listings: Arc<dyn PrivateRoomListingStore>,
    policies: Arc<dyn RoomContentPolicyLookup>,
    matrix: Arc<dyn PrivateRoomListingMatrix>,
    secrets: Arc<dyn SecretFactory>,
    clock: Arc<dyn Clock>,
}

/// 读到的这间房：快照、Matrix 房间、设置记录、此刻加没加密。
struct LoadedRoom {
    snapshot: PrivateRoomSnapshot,
    matrix_room: MatrixRoomId,
    listing: Option<PrivateRoomListingRecord>,
    encrypted: bool,
}

impl PrivateRoomListingService {
    pub fn new(dependencies: PrivateRoomListingDependencies) -> Self {
        Self {
            rooms: dependencies.rooms,
            listings: dependencies.listings,
            policies: dependencies.policies,
            matrix: dependencies.matrix,
            secrets: dependencies.secrets,
            clock: dependencies.clock,
        }
    }

    async fn inspect_internal(&self, request: InspectListing) -> ListingResult<ListingView> {
        const OPERATION: &str = "private_room.listing.inspect";
        let loaded = self
            .joined_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        Ok(view(loaded.listing.as_ref(), loaded.encrypted))
    }

    async fn update_internal(&self, request: UpdateListing) -> ListingResult<ListingView> {
        const OPERATION: &str = "private_room.listing.update";
        let loaded = self
            .joined_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        loaded
            .snapshot
            .room()
            .authorize_listing_change(request.actor.principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        if request.settings.listed() {
            self.list(&request, loaded, OPERATION).await
        } else {
            self.unlist(&request, loaded, OPERATION).await
        }
    }

    /// 公开，或者公开着改别的设置。
    async fn list(
        &self,
        request: &UpdateListing,
        loaded: LoadedRoom,
        operation: &'static str,
    ) -> ListingResult<ListingView> {
        let owner = request.actor.principal_id;
        let newly_listed = !loaded
            .listing
            .as_ref()
            .is_some_and(|listing| listing.settings.listed());
        if newly_listed {
            // 先查一遍限额，免得白写一条 Matrix 状态；存的时候在事务里还会再查。
            let listed = self
                .listings
                .listed_rooms_owned_by(owner)
                .await
                .map_err(|error| repository(operation, &error))?;
            if listed.iter().any(|catalog| *catalog != request.catalog_id) {
                return Err(ListingFailure::new(
                    operation,
                    ListingFailureKind::LimitReached,
                ));
            }
        }
        let now = self.clock.now();
        let first_listed_at = if let Some(listing) = &loaded.listing {
            listing.first_listed_at
        } else {
            // 第一次公开：之后进来的人不再拿到公开以前的房间钥匙。没写成就不公开。
            self.matrix
                .mark_listed(&loaded.matrix_room, now)
                .await
                .map_err(|error| matrix(operation, error))?;
            now
        };
        let mut record = PrivateRoomListingRecord {
            catalog_id: request.catalog_id,
            settings: request.settings,
            share_code: match &loaded.listing {
                Some(listing) => listing.share_code.clone(),
                None => self.new_share_code(operation)?,
            },
            first_listed_at,
            updated_by: owner,
            updated_at: now.max(first_listed_at),
            version: next_version(loaded.listing.as_ref(), operation)?,
        };
        let expected = loaded.listing.as_ref().map(|listing| listing.version);
        for _ in 0..SHARE_CODE_ATTEMPTS {
            match self.save(&record, expected, owner, operation).await? {
                PrivateRoomListingSaveOutcome::ShareCodeTaken if loaded.listing.is_none() => {
                    record.share_code = self.new_share_code(operation)?;
                }
                PrivateRoomListingSaveOutcome::Saved => {
                    return Ok(view(Some(&record), loaded.encrypted));
                }
                // 分享码没换也撞了是库里的数据不对；超限额和版本不对 `save` 已经报了错。
                PrivateRoomListingSaveOutcome::ShareCodeTaken
                | PrivateRoomListingSaveOutcome::OwnerLimitReached
                | PrivateRoomListingSaveOutcome::Stale => break,
            }
        }
        Err(ListingFailure::new(operation, ListingFailureKind::Internal))
    }

    /// 改回私密。不加密的房间先在 Matrix 上打开加密，再改目录上的标记，最后才存设置：哪一步没成，
    /// 房主再点一次都接得上（Matrix 上写一样的加密状态是幂等的）。
    async fn unlist(
        &self,
        request: &UpdateListing,
        loaded: LoadedRoom,
        operation: &'static str,
    ) -> ListingResult<ListingView> {
        let Some(previous) = loaded.listing else {
            // 从没公开过：没什么可改的，也不给没公开过的房间记设置。
            return Ok(view(None, loaded.encrypted));
        };
        if !loaded.encrypted {
            self.matrix
                .enable_encryption(&loaded.matrix_room)
                .await
                .map_err(|error| matrix(operation, error))?;
            self.listings
                .mark_room_encrypted(request.catalog_id)
                .await
                .map_err(|error| repository(operation, &error))?;
        }
        let record = PrivateRoomListingRecord {
            settings: request.settings,
            updated_by: request.actor.principal_id,
            updated_at: self.clock.now().max(previous.first_listed_at),
            version: next_version(Some(&previous), operation)?,
            ..previous.clone()
        };
        match self
            .save(
                &record,
                Some(previous.version),
                request.actor.principal_id,
                operation,
            )
            .await?
        {
            PrivateRoomListingSaveOutcome::Saved => Ok(view(Some(&record), true)),
            // 分享码没换，撞不上；真撞上了是库里的数据不对。
            PrivateRoomListingSaveOutcome::ShareCodeTaken
            | PrivateRoomListingSaveOutcome::OwnerLimitReached
            | PrivateRoomListingSaveOutcome::Stale => {
                Err(ListingFailure::new(operation, ListingFailureKind::Internal))
            }
        }
    }

    /// 存设置。超了限额、版本对不上直接报错；分享码撞了交给调用方换一个。
    async fn save(
        &self,
        record: &PrivateRoomListingRecord,
        expected: Option<AggregateVersion>,
        owner: PrincipalId,
        operation: &'static str,
    ) -> ListingResult<PrivateRoomListingSaveOutcome> {
        let outcome = self
            .listings
            .save_listing(record, expected, owner)
            .await
            .map_err(|error| repository(operation, &error))?;
        match outcome {
            PrivateRoomListingSaveOutcome::OwnerLimitReached => Err(ListingFailure::new(
                operation,
                ListingFailureKind::LimitReached,
            )),
            PrivateRoomListingSaveOutcome::Stale => {
                Err(ListingFailure::new(operation, ListingFailureKind::Conflict))
            }
            outcome => Ok(outcome),
        }
    }

    fn new_share_code(&self, operation: &'static str) -> ListingResult<RoomShareCode> {
        let random = self.secrets.generate().map_err(|_| {
            ListingFailure::new(operation, ListingFailureKind::DependencyUnavailable)
        })?;
        let mut entropy = [0_u8; 16];
        entropy.copy_from_slice(&self.secrets.digest(random.expose()).as_bytes()[..16]);
        Ok(RoomShareCode::from_entropy(entropy))
    }

    /// 登录有效、房间在、操作者已加入。看不到的和没有这个房间一样回答。
    async fn joined_room(
        &self,
        actor: &AuthenticatedPrincipal,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> ListingResult<LoadedRoom> {
        if self.clock.now() >= actor.expires_at {
            return Err(ListingFailure::new(
                operation,
                ListingFailureKind::Forbidden,
            ));
        }
        let not_found = || ListingFailure::new(operation, ListingFailureKind::NotFound);
        let snapshot = self
            .rooms
            .find_by_catalog(catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .filter(|snapshot| {
                snapshot
                    .room()
                    .member(actor.principal_id)
                    .is_some_and(PrivateRoomMember::has_joined)
            })
            .ok_or_else(not_found)?;
        let matrix_room =
            MatrixRoomId::new(snapshot.instance().matrix_room_id().as_str().to_owned())
                .map_err(|_| ListingFailure::new(operation, ListingFailureKind::Internal))?;
        let encrypted = self
            .policies
            .content_policy(&matrix_room)
            .await
            .map_err(|error| repository(operation, &error))?
            .ok_or_else(|| ListingFailure::new(operation, ListingFailureKind::Internal))?
            .end_to_end_encrypted;
        let listing = self
            .listings
            .listing(catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?;
        Ok(LoadedRoom {
            snapshot,
            matrix_room,
            listing,
            encrypted,
        })
    }
}

impl PrivateRoomListingUseCases for PrivateRoomListingService {
    fn inspect(&self, request: InspectListing) -> PortFuture<'_, ListingResult<ListingView>> {
        Box::pin(self.inspect_internal(request))
    }

    fn update(&self, request: UpdateListing) -> PortFuture<'_, ListingResult<ListingView>> {
        Box::pin(self.update_internal(request))
    }
}

fn view(listing: Option<&PrivateRoomListingRecord>, encrypted: bool) -> ListingView {
    ListingView {
        settings: listing.map_or(RoomListingSettings::UNLISTED, |listing| listing.settings),
        share_code: listing.map(|listing| listing.share_code.clone()),
        first_listed_at: listing.map(|listing| listing.first_listed_at),
        end_to_end_encrypted: encrypted,
    }
}

fn next_version(
    listing: Option<&PrivateRoomListingRecord>,
    operation: &'static str,
) -> ListingResult<AggregateVersion> {
    listing
        .map_or(AggregateVersion::INITIAL, |listing| listing.version)
        .next()
        .map_err(|error| domain(operation, &error))
}

const fn domain(operation: &'static str, error: &DomainError) -> ListingFailure {
    let kind = match error {
        DomainError::Forbidden { .. } => ListingFailureKind::Forbidden,
        DomainError::Validation { .. } => ListingFailureKind::InvalidRequest,
        DomainError::InvalidTransition { .. } => ListingFailureKind::Conflict,
        DomainError::InvariantViolation { .. }
        | DomainError::CapacityExceeded { .. }
        | DomainError::TimeOverflow
        | DomainError::VersionOverflow => ListingFailureKind::Internal,
    };
    ListingFailure::new(operation, kind)
}

const fn repository(operation: &'static str, error: &RepositoryError) -> ListingFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Unavailable => ListingFailureKind::DependencyUnavailable,
        RepositoryErrorKind::Forbidden => ListingFailureKind::Forbidden,
        RepositoryErrorKind::NotFound => ListingFailureKind::NotFound,
        RepositoryErrorKind::Conflict => ListingFailureKind::Conflict,
        RepositoryErrorKind::Constraint | RepositoryErrorKind::CorruptData => {
            ListingFailureKind::Internal
        }
    };
    ListingFailure::new(operation, kind)
}

/// Matrix 暂时写不进去的，房主过一会儿再点；别的是配置或数据不对。
const fn matrix(operation: &'static str, error: MatrixFailure) -> ListingFailure {
    let kind = match error.kind() {
        MatrixFailureKind::RateLimited
        | MatrixFailureKind::Timeout
        | MatrixFailureKind::DependencyUnavailable
        | MatrixFailureKind::UnknownCommit => ListingFailureKind::DependencyUnavailable,
        MatrixFailureKind::InvalidConfiguration
        | MatrixFailureKind::Unauthenticated
        | MatrixFailureKind::AuthenticationRejected
        | MatrixFailureKind::Forbidden
        | MatrixFailureKind::NotFound
        | MatrixFailureKind::Conflict
        | MatrixFailureKind::CryptographicIdentityConflict
        | MatrixFailureKind::InvalidResponse
        | MatrixFailureKind::StaleSyncToken
        | MatrixFailureKind::UnsupportedVersion => ListingFailureKind::Internal,
    };
    ListingFailure::new(operation, kind)
}
