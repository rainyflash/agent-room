mod agents;
mod audit;
mod content;
mod identity;
mod matrix;
mod notifications;
mod outbox;
mod projections;
mod rooms;
mod runtime;

use std::{future::Future, pin::Pin};

pub use agents::{AgentRegistration, AgentRegistrationTransaction, AgentRepository};
pub use audit::{AuditRecord, AuditSink};
pub use content::ContentStore;
pub use identity::{PrincipalRegistration, PrincipalRepository};
pub use matrix::{MatrixEvent, MatrixGateway};
pub use notifications::NotificationSink;
pub use outbox::{
    ClaimedOutboxEvent, OutboxBacklog, OutboxClaim, OutboxFailure, OutboxFailureOutcome,
    OutboxMessage, OutboxPublisher, OutboxRepository, PublishFailure, PublishFailureKind,
};
pub use projections::{
    ActivityScoreMillis, MatrixMembership, MatrixProjectionBatch, MatrixProjectionEvent,
    MatrixProjectionEventKind, MatrixProjectionRebuild, MatrixProjectionStore,
    MembershipProjectionLookup, MembershipReadPlan, ProjectionApplyOutcome, ProjectionCursor,
    ProjectionFreshnessPolicy, ProjectionHealth, ProjectionHealthReport,
};
pub use rooms::RoomDirectory;
pub use runtime::{Clock, IdentifierFactory};

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
