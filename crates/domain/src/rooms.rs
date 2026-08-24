use std::cmp::Ordering;

use crate::{
    DomainError, DomainResult,
    ids::{AgentInstanceId, PrincipalId, RoomCatalogId, RoomInstanceId, RoomReservationId},
    time::UtcMillis,
};

pub const DEFAULT_ROOM_SOFT_CAPACITY: u16 = 180;
pub const DEFAULT_ROOM_HARD_CAPACITY: u16 = 250;
const MAXIMUM_ROOM_CAPACITY: u16 = 1_000;
const MAXIMUM_ROOM_NAME_CHARACTERS: usize = 128;
const MAXIMUM_ROOM_DESCRIPTION_CHARACTERS: usize = 2_048;
const MAXIMUM_MATRIX_ROOM_ID_BYTES: usize = 512;
const MAXIMUM_REGION_HINT_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomCatalogKind {
    PublicLobby,
    PrivateRoom,
    Direct,
}

impl RoomCatalogKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PublicLobby => "public_lobby",
            Self::PrivateRoom => "private_room",
            Self::Direct => "direct",
        }
    }
}

impl TryFrom<&str> for RoomCatalogKind {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "public_lobby" => Ok(Self::PublicLobby),
            "private_room" => Ok(Self::PrivateRoom),
            "direct" => Ok(Self::Direct),
            _ => Err(validation("room_catalog_kind", "不是支持的目录类型")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomCatalogVisibility {
    Public,
    Unlisted,
    Private,
}

impl RoomCatalogVisibility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Unlisted => "unlisted",
            Self::Private => "private",
        }
    }
}

impl TryFrom<&str> for RoomCatalogVisibility {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "public" => Ok(Self::Public),
            "unlisted" => Ok(Self::Unlisted),
            "private" => Ok(Self::Private),
            _ => Err(validation(
                "room_catalog_visibility",
                "不是支持的目录可见性",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomCatalogStatus {
    Active,
    Frozen,
    Archived,
}

impl RoomCatalogStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Frozen => "frozen",
            Self::Archived => "archived",
        }
    }
}

impl TryFrom<&str> for RoomCatalogStatus {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "active" => Ok(Self::Active),
            "frozen" => Ok(Self::Frozen),
            "archived" => Ok(Self::Archived),
            _ => Err(validation("room_catalog_status", "不是支持的目录状态")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RoomSlug(String);

impl RoomSlug {
    /// 创建可放入公开目录 URL 的稳定短名。
    ///
    /// # Errors
    ///
    /// 值不是 1 到 63 字节的小写字母、数字和连字符组合时返回错误。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        let mut characters = value.chars();
        let Some(first) = characters.next() else {
            return Err(validation("room_slug", "不能为空"));
        };
        if value.len() > 63
            || !first.is_ascii_lowercase() && !first.is_ascii_digit()
            || !characters.all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
            })
        {
            return Err(validation(
                "room_slug",
                "必须是小写字母或数字开头的 URL 安全短名",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RoomLanguage(String);

impl RoomLanguage {
    /// 创建目录支持的 BCP 47 子集语言标签。
    ///
    /// # Errors
    ///
    /// 主语言或后续子标签超出边界时返回错误。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        let mut segments = value.split('-');
        let Some(primary) = segments.next() else {
            return Err(validation("room_language", "不能为空"));
        };
        if !(2..=8).contains(&primary.len())
            || !primary
                .chars()
                .all(|character| character.is_ascii_alphabetic())
            || !segments.all(|segment| {
                (1..=8).contains(&segment.len())
                    && segment
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric())
            })
        {
            return Err(validation(
                "room_language",
                "必须符合受支持的 BCP 47 标签子集",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RoomRegion(String);

impl RoomRegion {
    /// 创建只用于延迟偏好的地区提示。
    ///
    /// # Errors
    ///
    /// 值为空、超长或包含不安全字符时返回错误。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAXIMUM_REGION_HINT_BYTES
            || !value.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, '-' | '_')
            })
        {
            return Err(validation(
                "room_region",
                "必须是 1 到 64 字节的小写地区提示",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MatrixRoomReference(String);

impl MatrixRoomReference {
    /// 创建协议无关的 Matrix 房间或 Space 标识。
    ///
    /// # Errors
    ///
    /// 标识不符合 `!localpart:server` 的基本边界时返回错误。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        if value.len() < 4
            || value.len() > MAXIMUM_MATRIX_ROOM_ID_BYTES
            || !value.starts_with('!')
            || !value[1..].contains(':')
            || value.chars().any(char::is_control)
        {
            return Err(validation(
                "matrix_room_reference",
                "不是合法的 Matrix 房间标识",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomCatalogFields {
    pub kind: RoomCatalogKind,
    pub slug: Option<RoomSlug>,
    pub name: String,
    pub description: String,
    pub language: Option<RoomLanguage>,
    pub matrix_space_id: Option<MatrixRoomReference>,
    pub owner_principal_id: Option<PrincipalId>,
    pub visibility: RoomCatalogVisibility,
    pub retention_days: Option<u16>,
    pub status: RoomCatalogStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomCatalog {
    id: RoomCatalogId,
    fields: RoomCatalogFields,
}

impl RoomCatalog {
    /// 创建大厅目录条目并验证类型相关约束。
    ///
    /// # Errors
    ///
    /// 名称、说明、保留期或类型相关字段不满足领域不变式时返回错误。
    pub fn new(id: RoomCatalogId, fields: RoomCatalogFields) -> DomainResult<Self> {
        validate_room_text(
            "room_name",
            &fields.name,
            1,
            MAXIMUM_ROOM_NAME_CHARACTERS,
            false,
        )?;
        validate_room_text(
            "room_description",
            &fields.description,
            0,
            MAXIMUM_ROOM_DESCRIPTION_CHARACTERS,
            true,
        )?;
        if fields
            .retention_days
            .is_some_and(|days| days == 0 || days > 3_650)
        {
            return Err(validation(
                "room_retention_days",
                "必须处于 1 到 3650 天之间",
            ));
        }
        match fields.kind {
            RoomCatalogKind::PublicLobby => {
                if fields.slug.is_none() || fields.owner_principal_id.is_some() {
                    return Err(invariant(
                        "room_catalog",
                        "公共大厅必须有短名且不能绑定私人房主",
                    ));
                }
                if fields.visibility == RoomCatalogVisibility::Private {
                    return Err(invariant("room_catalog", "公共大厅不能使用私人可见性"));
                }
            }
            RoomCatalogKind::PrivateRoom => {
                if fields.owner_principal_id.is_none()
                    || fields.visibility == RoomCatalogVisibility::Public
                {
                    return Err(invariant(
                        "room_catalog",
                        "私人房间必须有房主且不能公开列出",
                    ));
                }
            }
            RoomCatalogKind::Direct => {
                if fields.slug.is_some()
                    || fields.matrix_space_id.is_some()
                    || fields.visibility != RoomCatalogVisibility::Private
                {
                    return Err(invariant(
                        "room_catalog",
                        "直接会话不能进入目录或挂载主题 Space",
                    ));
                }
            }
        }
        Ok(Self { id, fields })
    }

    pub const fn id(&self) -> RoomCatalogId {
        self.id
    }

    pub const fn kind(&self) -> RoomCatalogKind {
        self.fields.kind
    }

    pub fn slug(&self) -> Option<&RoomSlug> {
        self.fields.slug.as_ref()
    }

    pub fn name(&self) -> &str {
        &self.fields.name
    }

    pub fn description(&self) -> &str {
        &self.fields.description
    }

    pub fn language(&self) -> Option<&RoomLanguage> {
        self.fields.language.as_ref()
    }

    pub fn matrix_space_id(&self) -> Option<&MatrixRoomReference> {
        self.fields.matrix_space_id.as_ref()
    }

    pub const fn visibility(&self) -> RoomCatalogVisibility {
        self.fields.visibility
    }

    pub const fn status(&self) -> RoomCatalogStatus {
        self.fields.status
    }

    pub const fn is_joinable(&self) -> bool {
        matches!(self.fields.status, RoomCatalogStatus::Active)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomCapacity {
    soft: u16,
    hard: u16,
}

impl RoomCapacity {
    /// 创建软、硬容量阈值。
    ///
    /// # Errors
    ///
    /// 阈值为零、硬阈值不大于软阈值或超过系统上限时返回错误。
    pub fn new(soft: u16, hard: u16) -> DomainResult<Self> {
        if soft == 0 || hard <= soft || hard > MAXIMUM_ROOM_CAPACITY {
            return Err(validation(
                "room_capacity",
                "必须满足 0 < soft < hard <= 1000",
            ));
        }
        Ok(Self { soft, hard })
    }

    pub const fn standard() -> Self {
        Self {
            soft: DEFAULT_ROOM_SOFT_CAPACITY,
            hard: DEFAULT_ROOM_HARD_CAPACITY,
        }
    }

    pub const fn soft(self) -> u16 {
        self.soft
    }

    pub const fn hard(self) -> u16 {
        self.hard
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomInstanceState {
    Provisioning,
    Active,
    Draining,
    Archived,
    Failed,
}

impl RoomInstanceState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provisioning => "provisioning",
            Self::Active => "active",
            Self::Draining => "draining",
            Self::Archived => "archived",
            Self::Failed => "failed",
        }
    }

    pub const fn accepts_allocations(self) -> bool {
        matches!(self, Self::Active)
    }
}

impl TryFrom<&str> for RoomInstanceState {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "provisioning" => Ok(Self::Provisioning),
            "active" => Ok(Self::Active),
            "draining" => Ok(Self::Draining),
            "archived" => Ok(Self::Archived),
            "failed" => Ok(Self::Failed),
            _ => Err(validation("room_instance_state", "不是支持的实例状态")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomInstanceFields {
    pub catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomReference,
    pub region: Option<RoomRegion>,
    pub capacity: RoomCapacity,
    pub projected_member_count: u16,
    pub allocated_slots: u16,
    pub activity_score_millis: u64,
    pub state: RoomInstanceState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomInstance {
    id: RoomInstanceId,
    fields: RoomInstanceFields,
}

impl RoomInstance {
    /// 恢复一个房间容量快照。
    ///
    /// # Errors
    ///
    /// 投影人数或已分配槽位超过硬容量时返回错误。
    pub fn restore(id: RoomInstanceId, fields: RoomInstanceFields) -> DomainResult<Self> {
        if fields.projected_member_count > fields.capacity.hard()
            || fields.allocated_slots > fields.capacity.hard()
        {
            return Err(DomainError::CapacityExceeded {
                capacity: fields.capacity.hard(),
            });
        }
        Ok(Self { id, fields })
    }

    pub const fn id(&self) -> RoomInstanceId {
        self.id
    }

    pub const fn catalog_id(&self) -> RoomCatalogId {
        self.fields.catalog_id
    }

    pub const fn matrix_room_id(&self) -> &MatrixRoomReference {
        &self.fields.matrix_room_id
    }

    pub const fn region(&self) -> Option<&RoomRegion> {
        self.fields.region.as_ref()
    }

    pub const fn capacity(&self) -> RoomCapacity {
        self.fields.capacity
    }

    pub const fn projected_member_count(&self) -> u16 {
        self.fields.projected_member_count
    }

    pub const fn allocated_slots(&self) -> u16 {
        self.fields.allocated_slots
    }

    pub const fn activity_score_millis(&self) -> u64 {
        self.fields.activity_score_millis
    }

    pub const fn state(&self) -> RoomInstanceState {
        self.fields.state
    }

    pub const fn remaining_slots(&self) -> u16 {
        self.fields
            .capacity
            .hard()
            .saturating_sub(self.fields.allocated_slots)
    }

    pub const fn is_above_soft_capacity(&self) -> bool {
        self.fields.allocated_slots >= self.fields.capacity.soft()
    }

    pub const fn accepts_allocations(&self) -> bool {
        self.fields.state.accepts_allocations() && self.remaining_slots() > 0
    }

    /// 在已锁定快照上保留一个容量槽位。
    ///
    /// # Errors
    ///
    /// 房间不可分配或已经达到硬上限时返回错误。
    pub fn reserve_slot(&mut self) -> DomainResult<u16> {
        if !self.fields.state.accepts_allocations() {
            return Err(DomainError::InvalidTransition {
                entity: "room_instance",
                from: self.fields.state.as_str(),
                to: "reserved",
            });
        }
        if self.fields.allocated_slots >= self.fields.capacity.hard() {
            return Err(DomainError::CapacityExceeded {
                capacity: self.fields.capacity.hard(),
            });
        }
        self.fields.allocated_slots += 1;
        Ok(self.fields.allocated_slots)
    }

    /// 释放一个已存在的容量槽位。
    ///
    /// # Errors
    ///
    /// 没有槽位可释放时返回数据不变式错误。
    pub fn release_slot(&mut self) -> DomainResult<u16> {
        self.fields.allocated_slots = self
            .fields
            .allocated_slots
            .checked_sub(1)
            .ok_or_else(|| invariant("room_instance", "不能释放不存在的容量槽位"))?;
        Ok(self.fields.allocated_slots)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoomRecoveryAffinity {
    #[default]
    Other,
    Previous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoomInvitationAffinity {
    #[default]
    None,
    Explicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RoomPreferenceMatch {
    #[default]
    Different,
    Matching,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoomAllocationAffinity {
    pub recovery: RoomRecoveryAffinity,
    pub friends_in_room: u16,
    pub invitation: RoomInvitationAffinity,
    pub language: RoomPreferenceMatch,
    pub region: RoomPreferenceMatch,
}

impl RoomAllocationAffinity {
    const fn keeps_social_context(self) -> bool {
        matches!(self.recovery, RoomRecoveryAffinity::Previous)
            || self.friends_in_room > 0
            || matches!(self.invitation, RoomInvitationAffinity::Explicit)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomAllocationCandidate {
    instance: RoomInstance,
    affinity: RoomAllocationAffinity,
}

impl RoomAllocationCandidate {
    pub const fn new(instance: RoomInstance, affinity: RoomAllocationAffinity) -> Self {
        Self { instance, affinity }
    }

    pub const fn instance(&self) -> &RoomInstance {
        &self.instance
    }

    pub const fn affinity(&self) -> RoomAllocationAffinity {
        self.affinity
    }

    fn ranking_key(&self) -> (bool, u16, bool, bool, bool, bool, u16) {
        (
            matches!(self.affinity.recovery, RoomRecoveryAffinity::Previous),
            self.affinity.friends_in_room,
            matches!(self.affinity.invitation, RoomInvitationAffinity::Explicit),
            matches!(self.affinity.language, RoomPreferenceMatch::Matching),
            matches!(self.affinity.region, RoomPreferenceMatch::Matching),
            !self.instance.is_above_soft_capacity(),
            self.instance.remaining_slots(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomAllocationDecision {
    Reserve(RoomInstanceId),
    ProvisionNew,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomReservationState {
    Reserved,
    Committed,
    Released,
    Expired,
}

impl RoomReservationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Committed => "committed",
            Self::Released => "released",
            Self::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomReservation {
    id: RoomReservationId,
    catalog_id: RoomCatalogId,
    room_instance_id: RoomInstanceId,
    agent_instance_id: AgentInstanceId,
    reserved_at: UtcMillis,
    expires_at: UtcMillis,
    state: RoomReservationState,
    finalized_at: Option<UtcMillis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomReservationFields {
    pub catalog_id: RoomCatalogId,
    pub room_instance_id: RoomInstanceId,
    pub agent_instance_id: AgentInstanceId,
    pub reserved_at: UtcMillis,
    pub expires_at: UtcMillis,
    pub state: RoomReservationState,
    pub finalized_at: Option<UtcMillis>,
}

impl RoomReservation {
    /// 创建一个等待 Matrix 加入确认的短期容量预约。
    ///
    /// # Errors
    ///
    /// 到期时间不晚于创建时间时返回错误。
    pub fn reserve(
        id: RoomReservationId,
        catalog_id: RoomCatalogId,
        room_instance_id: RoomInstanceId,
        agent_instance_id: AgentInstanceId,
        reserved_at: UtcMillis,
        expires_at: UtcMillis,
    ) -> DomainResult<Self> {
        if expires_at <= reserved_at {
            return Err(validation(
                "room_reservation_expires_at",
                "必须晚于预约时间",
            ));
        }
        Ok(Self {
            id,
            catalog_id,
            room_instance_id,
            agent_instance_id,
            reserved_at,
            expires_at,
            state: RoomReservationState::Reserved,
            finalized_at: None,
        })
    }

    /// 从持久化事实恢复预约。
    ///
    /// # Errors
    ///
    /// 时间顺序与状态终结时间不一致时返回错误。
    pub fn restore(id: RoomReservationId, fields: RoomReservationFields) -> DomainResult<Self> {
        let mut reservation = Self::reserve(
            id,
            fields.catalog_id,
            fields.room_instance_id,
            fields.agent_instance_id,
            fields.reserved_at,
            fields.expires_at,
        )?;
        let expects_finalized = !matches!(fields.state, RoomReservationState::Reserved);
        if expects_finalized != fields.finalized_at.is_some()
            || fields
                .finalized_at
                .is_some_and(|time| time < fields.reserved_at)
        {
            return Err(invariant("room_reservation", "终结状态与终结时间必须一致"));
        }
        reservation.state = fields.state;
        reservation.finalized_at = fields.finalized_at;
        Ok(reservation)
    }

    pub const fn id(&self) -> RoomReservationId {
        self.id
    }

    pub const fn catalog_id(&self) -> RoomCatalogId {
        self.catalog_id
    }

    pub const fn room_instance_id(&self) -> RoomInstanceId {
        self.room_instance_id
    }

    pub const fn agent_instance_id(&self) -> AgentInstanceId {
        self.agent_instance_id
    }

    pub const fn reserved_at(&self) -> UtcMillis {
        self.reserved_at
    }

    pub const fn expires_at(&self) -> UtcMillis {
        self.expires_at
    }

    pub const fn state(&self) -> RoomReservationState {
        self.state
    }

    pub const fn finalized_at(&self) -> Option<UtcMillis> {
        self.finalized_at
    }

    /// 确认 Matrix 已成功加入并把预约转成当前分配。
    ///
    /// # Errors
    ///
    /// 预约已释放、过期或确认时间越界时返回错误。
    pub fn commit(&mut self, now: UtcMillis) -> DomainResult<bool> {
        match self.state {
            RoomReservationState::Reserved if now < self.expires_at && now >= self.reserved_at => {
                self.state = RoomReservationState::Committed;
                self.finalized_at = Some(now);
                Ok(true)
            }
            RoomReservationState::Committed => Ok(false),
            RoomReservationState::Reserved => Err(DomainError::InvalidTransition {
                entity: "room_reservation",
                from: "reserved",
                to: "committed_after_expiry",
            }),
            RoomReservationState::Released | RoomReservationState::Expired => {
                Err(DomainError::InvalidTransition {
                    entity: "room_reservation",
                    from: self.state.as_str(),
                    to: "committed",
                })
            }
        }
    }

    /// 释放预约或当前分配；重复释放保持幂等。
    ///
    /// # Errors
    ///
    /// 时间早于预约时间时返回错误。
    pub fn release(&mut self, now: UtcMillis) -> DomainResult<bool> {
        if now < self.reserved_at {
            return Err(validation(
                "room_reservation_released_at",
                "不能早于预约时间",
            ));
        }
        match self.state {
            RoomReservationState::Reserved | RoomReservationState::Committed => {
                self.state = RoomReservationState::Released;
                self.finalized_at = Some(now);
                Ok(true)
            }
            RoomReservationState::Released | RoomReservationState::Expired => Ok(false),
        }
    }

    /// 在预约租期结束后回收未确认槽位。
    ///
    /// # Errors
    ///
    /// 在到期前尝试过期预约时返回错误。
    pub fn expire(&mut self, now: UtcMillis) -> DomainResult<bool> {
        match self.state {
            RoomReservationState::Reserved if now >= self.expires_at => {
                self.state = RoomReservationState::Expired;
                self.finalized_at = Some(now);
                Ok(true)
            }
            RoomReservationState::Reserved => Err(DomainError::InvalidTransition {
                entity: "room_reservation",
                from: "reserved",
                to: "expired_before_deadline",
            }),
            RoomReservationState::Committed
            | RoomReservationState::Released
            | RoomReservationState::Expired => Ok(false),
        }
    }
}

/// 根据恢复、社交、语言、地区和容量优先级选择大厅实例。
///
/// 没有可用实例，或最优普通实例已经达到软阈值时返回 `ProvisionNew`。恢复、好友同房和明确邀请
/// 可以在硬阈值前继续使用原实例，以免为了负载均衡破坏用户明确的社交上下文。
pub fn choose_room_instance(candidates: &[RoomAllocationCandidate]) -> RoomAllocationDecision {
    let best = candidates
        .iter()
        .filter(|candidate| candidate.instance.accepts_allocations())
        .max_by(|left, right| compare_candidates(left, right));
    let Some(best) = best else {
        return RoomAllocationDecision::ProvisionNew;
    };
    if best.instance.is_above_soft_capacity() && !best.affinity.keeps_social_context() {
        RoomAllocationDecision::ProvisionNew
    } else {
        RoomAllocationDecision::Reserve(best.instance.id())
    }
}

/// 验证用户手动指定的实例仍可加入。
///
/// # Errors
///
/// 目标不活跃或已经达到硬容量时返回错误。
pub fn choose_manual_room_instance(
    candidate: &RoomAllocationCandidate,
) -> DomainResult<RoomAllocationDecision> {
    if !candidate.instance.state().accepts_allocations() {
        return Err(DomainError::InvalidTransition {
            entity: "room_instance",
            from: candidate.instance.state().as_str(),
            to: "reserved",
        });
    }
    if candidate.instance.remaining_slots() == 0 {
        return Err(DomainError::CapacityExceeded {
            capacity: candidate.instance.capacity().hard(),
        });
    }
    Ok(RoomAllocationDecision::Reserve(candidate.instance.id()))
}

fn compare_candidates(left: &RoomAllocationCandidate, right: &RoomAllocationCandidate) -> Ordering {
    left.ranking_key()
        .cmp(&right.ranking_key())
        .then_with(|| right.instance.id().cmp(&left.instance.id()))
}

fn validate_room_text(
    field: &'static str,
    value: &str,
    minimum: usize,
    maximum: usize,
    allow_line_breaks: bool,
) -> DomainResult<()> {
    let character_count = value.chars().count();
    if !(minimum..=maximum).contains(&character_count)
        || value
            .chars()
            .any(|character| character.is_control() && !(allow_line_breaks && character == '\n'))
    {
        return Err(validation(field, "长度越界或包含控制字符"));
    }
    Ok(())
}

const fn validation(field: &'static str, reason: &'static str) -> DomainError {
    DomainError::Validation { field, reason }
}

const fn invariant(entity: &'static str, rule: &'static str) -> DomainError {
    DomainError::InvariantViolation { entity, rule }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::{
        MatrixRoomReference, RoomAllocationAffinity, RoomAllocationCandidate,
        RoomAllocationDecision, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState, RoomLanguage, RoomPreferenceMatch, RoomRecoveryAffinity, RoomSlug,
        choose_manual_room_instance, choose_room_instance,
    };
    use crate::{
        ids::{AgentInstanceId, PrincipalId, RoomCatalogId, RoomInstanceId, RoomReservationId},
        time::UtcMillis,
    };

    #[test]
    fn 公共大厅拒绝私人可见性和缺失短名() {
        let fields = RoomCatalogFields {
            kind: RoomCatalogKind::PublicLobby,
            slug: None,
            name: "综合大厅".to_owned(),
            description: String::new(),
            language: Some(RoomLanguage::new("zh-CN").expect("语言有效")),
            matrix_space_id: None,
            owner_principal_id: None,
            visibility: RoomCatalogVisibility::Private,
            retention_days: Some(30),
            status: RoomCatalogStatus::Active,
        };

        assert!(RoomCatalog::new(catalog_id(), fields).is_err());
    }

    #[test]
    fn 私人房间必须绑定房主且不会公开列出() {
        let valid = RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: Some(RoomSlug::new("private-team").expect("短名有效")),
            name: "项目房间".to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(PrincipalId::from_uuid(Uuid::now_v7())),
            visibility: RoomCatalogVisibility::Unlisted,
            retention_days: None,
            status: RoomCatalogStatus::Active,
        };
        assert!(RoomCatalog::new(catalog_id(), valid.clone()).is_ok());

        let invalid = RoomCatalogFields {
            owner_principal_id: None,
            ..valid
        };
        assert!(RoomCatalog::new(catalog_id(), invalid).is_err());
    }

    #[test]
    fn 自动分配优先恢复和好友而不是纯容量均衡() {
        let previous = candidate(
            1,
            200,
            RoomAllocationAffinity {
                recovery: RoomRecoveryAffinity::Previous,
                ..RoomAllocationAffinity::default()
            },
        );
        let friend = candidate(
            2,
            20,
            RoomAllocationAffinity {
                friends_in_room: 2,
                ..RoomAllocationAffinity::default()
            },
        );
        let empty = candidate(3, 0, RoomAllocationAffinity::default());

        assert_eq!(
            choose_room_instance(&[empty, friend, previous]),
            RoomAllocationDecision::Reserve(instance_id(1))
        );
    }

    #[test]
    fn 没有社交锚点时软阈值触发新分片() {
        let language_match = candidate(
            1,
            180,
            RoomAllocationAffinity {
                language: RoomPreferenceMatch::Matching,
                region: RoomPreferenceMatch::Matching,
                ..RoomAllocationAffinity::default()
            },
        );

        assert_eq!(
            choose_room_instance(&[language_match]),
            RoomAllocationDecision::ProvisionNew
        );
    }

    #[test]
    fn 手动切换不受软阈值限制但绝不突破硬阈值() {
        let below_hard = candidate(1, 249, RoomAllocationAffinity::default());
        assert_eq!(
            choose_manual_room_instance(&below_hard).expect("仍有一个槽位"),
            RoomAllocationDecision::Reserve(instance_id(1))
        );

        let full = candidate(2, 250, RoomAllocationAffinity::default());
        assert!(choose_manual_room_instance(&full).is_err());
    }

    #[test]
    fn 加入失败可释放预约且重复补偿保持幂等() {
        let mut reservation = super::RoomReservation::reserve(
            RoomReservationId::from_uuid(Uuid::now_v7()),
            catalog_id(),
            instance_id(1),
            AgentInstanceId::from_uuid(Uuid::now_v7()),
            time(1_000),
            time(61_000),
        )
        .expect("预约有效");

        assert!(reservation.release(time(2_000)).expect("首次释放成功"));
        assert!(!reservation.release(time(3_000)).expect("重复释放幂等"));
        assert_eq!(reservation.state(), super::RoomReservationState::Released);
    }

    #[test]
    fn 未确认预约只有到期后才能回收() {
        let mut reservation = super::RoomReservation::reserve(
            RoomReservationId::from_uuid(Uuid::now_v7()),
            catalog_id(),
            instance_id(1),
            AgentInstanceId::from_uuid(Uuid::now_v7()),
            time(1_000),
            time(61_000),
        )
        .expect("预约有效");

        assert!(reservation.expire(time(60_999)).is_err());
        assert!(reservation.expire(time(61_000)).expect("到期可回收"));
        assert_eq!(reservation.state(), super::RoomReservationState::Expired);
    }

    proptest! {
        #[test]
        fn 任意保留释放序列都不能突破硬容量(
            operations in prop::collection::vec(any::<bool>(), 0..2_000)
        ) {
            let mut room = room(1, 0);
            for reserve in operations {
                if reserve {
                    let _ = room.reserve_slot();
                } else if room.allocated_slots() > 0 {
                    room.release_slot().expect("已有槽位可以释放");
                }
                prop_assert!(room.allocated_slots() <= room.capacity().hard());
            }
        }
    }

    fn candidate(
        sequence: u128,
        allocated_slots: u16,
        affinity: RoomAllocationAffinity,
    ) -> RoomAllocationCandidate {
        RoomAllocationCandidate::new(room(sequence, allocated_slots), affinity)
    }

    fn room(sequence: u128, allocated_slots: u16) -> RoomInstance {
        RoomInstance::restore(
            instance_id(sequence),
            RoomInstanceFields {
                catalog_id: catalog_id(),
                matrix_room_id: MatrixRoomReference::new(format!("!room{sequence}:matrix.test"))
                    .expect("Matrix 房间 ID 有效"),
                region: None,
                capacity: RoomCapacity::standard(),
                projected_member_count: allocated_slots,
                allocated_slots,
                activity_score_millis: 0,
                state: RoomInstanceState::Active,
            },
        )
        .expect("房间快照有效")
    }

    fn catalog_id() -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::from_u128(10))
    }

    fn instance_id(sequence: u128) -> RoomInstanceId {
        RoomInstanceId::from_uuid(Uuid::from_u128(sequence))
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
