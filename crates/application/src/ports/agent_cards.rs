use agent_room_domain::{
    agent_cards::{
        AgentCardDigest, AgentCardSnapshot, AgentCardSourceUrl, AgentCardVerificationState,
        NormalizedAgentCard,
    },
    ids::AgentId,
    time::DurationMillis,
};
use thiserror::Error;

use crate::persistence::RepositoryResult;

use super::PortFuture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedAgentCard {
    pub digest: AgentCardDigest,
    pub card: NormalizedAgentCard,
    pub verification: AgentCardVerificationState,
    pub cache_lifetime: DurationMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCardFetchFailureKind {
    RejectedSource,
    BlockedNetworkTarget,
    InvalidResponse,
    UnsupportedProtocol,
    InvalidSignature,
    Unavailable,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Agent Card 获取操作 {operation} 失败：{kind:?}")]
pub struct AgentCardFetchFailure {
    operation: &'static str,
    kind: AgentCardFetchFailureKind,
}

impl AgentCardFetchFailure {
    pub const fn new(operation: &'static str, kind: AgentCardFetchFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> AgentCardFetchFailureKind {
        self.kind
    }
}

pub type AgentCardFetchResult<T> = Result<T, AgentCardFetchFailure>;

pub trait AgentCardSource: Send + Sync {
    fn fetch<'a>(
        &'a self,
        source_url: &'a AgentCardSourceUrl,
    ) -> PortFuture<'a, AgentCardFetchResult<FetchedAgentCard>>;
}

pub trait AgentCardSnapshotRepository: Send + Sync {
    fn find_latest(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentCardSnapshot>>>;

    fn save<'a>(
        &'a self,
        snapshot: &'a AgentCardSnapshot,
    ) -> PortFuture<'a, RepositoryResult<AgentCardSnapshot>>;
}
