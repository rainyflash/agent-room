//! 分享一段对话（`specs/redesign/project-rooms.md`“分享一段对话”，3P-7a）：
//! `POST`、`GET /private-rooms/{c}/excerpts`，`DELETE /private-rooms/{c}/excerpts/{code}`。
//!
//! - 只有放着实况的房间能分享一段（和分享页 `/r/<码>/live` 认的是同几样），房间里已加入的人都能分享。
//!   分享出去的本来就是不登录也看得到的话，不多漏一句。
//! - 只认开了实况以后说的话，开头和结尾都在主房间里，最多 30 句、连着的。几句、连不连着，照分享页实况
//!   同一套解析数：以应用服务从最新往回读这间房（最多 500 条事件）到开头那一句，撤回、隐藏的不算。
//! - 只存是哪几句（事件 ID）、第一句的时间、谁分享的、什么时候，不存话。
//! - 每人每小时最多分享 20 段（读房间之前就算一次）。
//! - 列：自己分享的；房主和管理员看这间房的全部，看不到是谁分享的（回答里只说是不是自己的）。
//! - 删：分享的人自己（离开了房间也能删），房主和管理员。
//! - 每天清一次（[`RoomExcerptCleanupService`]）：第一句过了房间保留期的、房间没在放实况的。关实况、删账户
//!   时库里在同一个事务里删（`postgres-adapter` 的 `rooms/excerpts.rs`）。

use std::sync::Arc;

use agent_room_domain::{
    ids::RoomCatalogId,
    private_rooms::{PrivateRoomCapability, PrivateRoomMember},
    room_excerpts::{MAX_EXCERPT_MESSAGES, RoomExcerptCode},
    room_listing::RoomShareCode,
    time::UtcMillis,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    hourly_limit::HourlyLimit,
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, ExcerptMessageRef, ExcerptTimelineRead, InsertRoomExcerptOutcome, MatrixEventId,
        PortFuture, PrivateRoomSnapshot, PrivateRoomStore, RoomExcerptRecord, RoomExcerptStore,
        RoomExcerptTimeline, SecretFactory,
    },
};

/// 每人每小时最多分享这么多段。
pub const SHARES_PER_HOUR: usize = 20;
/// 房间没设保留期时按这么多天算，和聊天服务器的默认一样。
pub const DEFAULT_RETENTION_DAYS: u16 = 30;
/// 段码撞上别的时最多换这么多次。
const CODE_ATTEMPTS: usize = 3;

/// 选的那几句哪里不对。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcerptRangeProblem {
    /// 最近 500 条事件里没找到开头或结尾那一句，结尾在开头前面，或者那一句撤回了、被隐藏了。
    NotFound,
    /// 有一句是开实况以前说的。
    BeforeLive,
}

impl ExcerptRangeProblem {
    /// 错误详情里的写法。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::BeforeLive => "before_live",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcerptFailureKind {
    /// 登录过期，或者不能删别人分享的。
    Forbidden,
    /// 没有这个房间、他不在房间里，或者没有这一段。
    NotFound,
    /// 房间此刻没在放实况。
    NotLive,
    /// 超过 30 句。
    TooLong,
    InvalidRange(ExcerptRangeProblem),
    /// 这一小时分享够了，隔这么多秒再来。
    RateLimited {
        retry_after_seconds: u64,
    },
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExcerptFailure {
    operation: &'static str,
    kind: ExcerptFailureKind,
}

impl ExcerptFailure {
    pub const fn new(operation: &'static str, kind: ExcerptFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> ExcerptFailureKind {
        self.kind
    }
}

pub type ExcerptResult<T> = Result<T, ExcerptFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareExcerpt {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub first_event_id: MatrixEventId,
    pub last_event_id: MatrixEventId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListExcerpts {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteExcerpt {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub code: RoomExcerptCode,
}

/// 刚分享的一段：链接是 `/r/<share_code>/s/<code>`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedExcerpt {
    pub code: RoomExcerptCode,
    pub share_code: RoomShareCode,
    pub message_count: usize,
}

/// 列表里的一段。不写是谁分享的，只说是不是问的人自己。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcerptSummary {
    pub code: RoomExcerptCode,
    pub message_count: usize,
    pub first_message_at: UtcMillis,
    pub shared_at: UtcMillis,
    pub mine: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcerptList {
    /// 这间房此刻的分享码，拼链接用；没有的话链接拼不出来。
    pub share_code: Option<RoomShareCode>,
    /// 新分享的在前。
    pub excerpts: Vec<ExcerptSummary>,
}

pub trait RoomExcerptUseCases: Send + Sync {
    fn share(&self, request: ShareExcerpt) -> PortFuture<'_, ExcerptResult<SharedExcerpt>>;

    fn list(&self, request: ListExcerpts) -> PortFuture<'_, ExcerptResult<ExcerptList>>;

    /// 已经删掉了的（别人刚删）也算删成。
    fn delete(&self, request: DeleteExcerpt) -> PortFuture<'_, ExcerptResult<()>>;
}

pub struct RoomExcerptDependencies {
    pub rooms: Arc<dyn PrivateRoomStore>,
    pub excerpts: Arc<dyn RoomExcerptStore>,
    /// 以应用服务读房间，看那几句在不在、连不连着。
    pub timeline: Arc<dyn RoomExcerptTimeline>,
    pub secrets: Arc<dyn SecretFactory>,
    pub clock: Arc<dyn Clock>,
}

pub struct RoomExcerptService {
    rooms: Arc<dyn PrivateRoomStore>,
    excerpts: Arc<dyn RoomExcerptStore>,
    timeline: Arc<dyn RoomExcerptTimeline>,
    secrets: Arc<dyn SecretFactory>,
    clock: Arc<dyn Clock>,
    shares: HourlyLimit,
}

impl RoomExcerptService {
    pub fn new(dependencies: RoomExcerptDependencies) -> Self {
        Self {
            rooms: dependencies.rooms,
            excerpts: dependencies.excerpts,
            timeline: dependencies.timeline,
            secrets: dependencies.secrets,
            clock: dependencies.clock,
            shares: HourlyLimit::new(SHARES_PER_HOUR),
        }
    }

    async fn share_internal(&self, request: ShareExcerpt) -> ExcerptResult<SharedExcerpt> {
        const OPERATION: &str = "room_excerpt.share";
        self.joined_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        let room = self
            .excerpts
            .excerpt_room(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .ok_or(ExcerptFailure::new(OPERATION, ExcerptFailureKind::NotLive))?;
        // 读房间之前就算一次：选错了的也算，免得拿它一遍遍读聊天服务器。
        let now = self.clock.now();
        self.shares
            .take(request.actor.principal_id, now)
            .map_err(|reached| {
                ExcerptFailure::new(
                    OPERATION,
                    ExcerptFailureKind::RateLimited {
                        retry_after_seconds: seconds_until(reached.retry_at, now),
                    },
                )
            })?;
        let read = self
            .timeline
            .visible_messages(&room.matrix_room_id, &request.first_event_id)
            .await
            .map_err(|_| {
                ExcerptFailure::new(OPERATION, ExcerptFailureKind::DependencyUnavailable)
            })?;
        let messages = match read {
            ExcerptTimelineRead::Found(messages) => messages,
            ExcerptTimelineRead::NotReachable => {
                return Err(ExcerptFailure::new(
                    OPERATION,
                    ExcerptFailureKind::InvalidRange(ExcerptRangeProblem::NotFound),
                ));
            }
            ExcerptTimelineRead::Encrypted => {
                return Err(ExcerptFailure::new(OPERATION, ExcerptFailureKind::NotLive));
            }
        };
        let range = excerpt_range(
            &messages,
            &request.first_event_id,
            &request.last_event_id,
            room.enabled_at,
        )
        .map_err(|kind| ExcerptFailure::new(OPERATION, kind))?;
        let mut record = RoomExcerptRecord {
            code: self.new_code(OPERATION)?,
            catalog_id: request.catalog_id,
            event_ids: range
                .iter()
                .map(|message| message.event_id.clone())
                .collect(),
            first_event_at: range[0].sent_at,
            shared_by: request.actor.principal_id,
            shared_at: self.clock.now(),
        };
        self.store(&mut record, room.enabled_at, OPERATION).await?;
        Ok(SharedExcerpt {
            code: record.code,
            share_code: room.share_code,
            message_count: record.event_ids.len(),
        })
    }

    /// 记下来；段码撞上了换一个再记。
    async fn store(
        &self,
        record: &mut RoomExcerptRecord,
        enabled_at: UtcMillis,
        operation: &'static str,
    ) -> ExcerptResult<()> {
        for attempt in 1..=CODE_ATTEMPTS {
            let outcome = self
                .excerpts
                .insert_excerpt(record, enabled_at)
                .await
                .map_err(|error| repository(operation, &error))?;
            match outcome {
                InsertRoomExcerptOutcome::Inserted => return Ok(()),
                InsertRoomExcerptOutcome::NotLive => {
                    return Err(ExcerptFailure::new(operation, ExcerptFailureKind::NotLive));
                }
                InsertRoomExcerptOutcome::CodeTaken if attempt < CODE_ATTEMPTS => {
                    record.code = self.new_code(operation)?;
                }
                InsertRoomExcerptOutcome::CodeTaken => {}
            }
        }
        Err(ExcerptFailure::new(operation, ExcerptFailureKind::Internal))
    }

    async fn list_internal(&self, request: ListExcerpts) -> ExcerptResult<ExcerptList> {
        const OPERATION: &str = "room_excerpt.list";
        let snapshot = self
            .joined_room(&request.actor, request.catalog_id, OPERATION)
            .await?;
        let actor = request.actor.principal_id;
        let manages = snapshot.room().allows(actor, PrivateRoomCapability::Manage);
        let listing = self
            .excerpts
            .room_excerpts(request.catalog_id, (!manages).then_some(actor))
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(ExcerptList {
            share_code: listing.share_code,
            excerpts: listing
                .excerpts
                .into_iter()
                .map(|excerpt| ExcerptSummary {
                    message_count: excerpt.event_ids.len(),
                    first_message_at: excerpt.first_event_at,
                    shared_at: excerpt.shared_at,
                    mine: excerpt.shared_by == actor,
                    code: excerpt.code,
                })
                .collect(),
        })
    }

    async fn delete_internal(&self, request: DeleteExcerpt) -> ExcerptResult<()> {
        const OPERATION: &str = "room_excerpt.delete";
        self.active_session(&request.actor, OPERATION)?;
        let excerpt = self
            .excerpts
            .find_excerpt(request.catalog_id, &request.code)
            .await
            .map_err(|error| repository(OPERATION, &error))?
            .ok_or(ExcerptFailure::new(OPERATION, ExcerptFailureKind::NotFound))?;
        if excerpt.shared_by != request.actor.principal_id {
            let snapshot = self
                .joined_room(&request.actor, request.catalog_id, OPERATION)
                .await?;
            if !snapshot
                .room()
                .allows(request.actor.principal_id, PrivateRoomCapability::Manage)
            {
                return Err(ExcerptFailure::new(
                    OPERATION,
                    ExcerptFailureKind::Forbidden,
                ));
            }
        }
        self.excerpts
            .delete_excerpt(request.catalog_id, &request.code)
            .await
            .map_err(|error| repository(OPERATION, &error))?;
        Ok(())
    }

    fn new_code(&self, operation: &'static str) -> ExcerptResult<RoomExcerptCode> {
        random_excerpt_code(self.secrets.as_ref())
            .ok_or(ExcerptFailure::new(operation, ExcerptFailureKind::Internal))
    }

    fn active_session(
        &self,
        actor: &AuthenticatedPrincipal,
        operation: &'static str,
    ) -> ExcerptResult<()> {
        if self.clock.now() >= actor.expires_at {
            return Err(ExcerptFailure::new(
                operation,
                ExcerptFailureKind::Forbidden,
            ));
        }
        Ok(())
    }

    /// 登录有效、房间在、操作者已加入。看不到的和没有这个房间一样回答。
    async fn joined_room(
        &self,
        actor: &AuthenticatedPrincipal,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> ExcerptResult<PrivateRoomSnapshot> {
        self.active_session(actor, operation)?;
        self.rooms
            .find_by_catalog(catalog_id)
            .await
            .map_err(|error| repository(operation, &error))?
            .filter(|snapshot| {
                snapshot
                    .room()
                    .member(actor.principal_id)
                    .is_some_and(PrivateRoomMember::has_joined)
            })
            .ok_or(ExcerptFailure::new(operation, ExcerptFailureKind::NotFound))
    }
}

impl RoomExcerptUseCases for RoomExcerptService {
    fn share(&self, request: ShareExcerpt) -> PortFuture<'_, ExcerptResult<SharedExcerpt>> {
        Box::pin(self.share_internal(request))
    }

    fn list(&self, request: ListExcerpts) -> PortFuture<'_, ExcerptResult<ExcerptList>> {
        Box::pin(self.list_internal(request))
    }

    fn delete(&self, request: DeleteExcerpt) -> PortFuture<'_, ExcerptResult<()>> {
        Box::pin(self.delete_internal(request))
    }
}

/// 每天清一次存着的段。
pub trait RoomExcerptCleanupUseCases: Send + Sync {
    /// 回删了几段。
    fn prune(&self) -> PortFuture<'_, ExcerptResult<u64>>;
}

pub struct RoomExcerptCleanupService {
    excerpts: Arc<dyn RoomExcerptStore>,
    clock: Arc<dyn Clock>,
}

impl RoomExcerptCleanupService {
    pub fn new(excerpts: Arc<dyn RoomExcerptStore>, clock: Arc<dyn Clock>) -> Self {
        Self { excerpts, clock }
    }
}

impl RoomExcerptCleanupUseCases for RoomExcerptCleanupService {
    fn prune(&self) -> PortFuture<'_, ExcerptResult<u64>> {
        Box::pin(async move {
            self.excerpts
                .prune_excerpts(self.clock.now(), DEFAULT_RETENTION_DAYS)
                .await
                .map_err(|error| repository("room_excerpt.prune", &error))
        })
    }
}

/// 从开头那一句到结尾那一句（都算上）。读到的话里找不到、结尾在开头前面、超过 30 句、有一句早于开实况
/// 那一刻，都不行。
fn excerpt_range<'a>(
    messages: &'a [ExcerptMessageRef],
    first: &MatrixEventId,
    last: &MatrixEventId,
    enabled_at: UtcMillis,
) -> Result<&'a [ExcerptMessageRef], ExcerptFailureKind> {
    let not_found = ExcerptFailureKind::InvalidRange(ExcerptRangeProblem::NotFound);
    let start = messages
        .iter()
        .position(|message| message.event_id == *first)
        .ok_or(not_found)?;
    let end = messages[start..]
        .iter()
        .position(|message| message.event_id == *last)
        .map(|offset| start + offset)
        .ok_or(not_found)?;
    let range = &messages[start..=end];
    if range.len() > MAX_EXCERPT_MESSAGES {
        return Err(ExcerptFailureKind::TooLong);
    }
    if range.iter().any(|message| message.sent_at < enabled_at) {
        return Err(ExcerptFailureKind::InvalidRange(
            ExcerptRangeProblem::BeforeLive,
        ));
    }
    Ok(range)
}

/// 新的段码，取安全随机数的前 16 字节。随机源不可用时是 `None`。
fn random_excerpt_code(secrets: &dyn SecretFactory) -> Option<RoomExcerptCode> {
    let random = secrets.generate().ok()?;
    let mut entropy = [0_u8; 16];
    entropy.copy_from_slice(&secrets.digest(random.expose()).as_bytes()[..16]);
    Some(RoomExcerptCode::from_entropy(entropy))
}

/// 到 `retry_at` 还有几秒，往上取整，至少 1 秒。
fn seconds_until(retry_at: UtcMillis, now: UtcMillis) -> u64 {
    let millis = u64::try_from(retry_at.value().saturating_sub(now.value())).unwrap_or(0);
    millis.div_ceil(1_000).max(1)
}

const fn repository(operation: &'static str, error: &RepositoryError) -> ExcerptFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Unavailable => ExcerptFailureKind::DependencyUnavailable,
        RepositoryErrorKind::NotFound => ExcerptFailureKind::NotFound,
        RepositoryErrorKind::Conflict
        | RepositoryErrorKind::Forbidden
        | RepositoryErrorKind::Constraint
        | RepositoryErrorKind::CorruptData => ExcerptFailureKind::Internal,
    };
    ExcerptFailure::new(operation, kind)
}
