use std::sync::Arc;

use agent_room_domain::{
    ids::AuditEventId,
    moderation::{
        ModerationAction, ModerationActionStatus, ModerationAuditEvent, ModerationAuditOutcome,
        ModerationCase, ModerationRole,
    },
};

use crate::{
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, MatrixFailure, ModerationActionReservationOutcome, ModerationAuthority,
        ModerationEffectGateway, ModerationEffectTarget, ModerationIdentifierFactory,
        ModerationReportPolicy, ModerationReportSubmissionOutcome, ModerationRepository,
        ModerationRoomContext, PortFuture,
    },
};

use super::{
    ApplyModerationAction, ListModerationAudit, ListMyModerationCases, ListRoomModeration,
    ModerationFailure, ModerationFailureKind, ModerationResult, ReverseModerationAction,
    SubmitModerationReport,
};

const MAXIMUM_AUDIT_PAGE: u16 = 200;

pub trait ModerationUseCases: Send + Sync {
    fn submit_report(
        &self,
        request: SubmitModerationReport,
    ) -> PortFuture<'_, ModerationResult<ModerationCase>>;

    fn list_my_cases(
        &self,
        request: ListMyModerationCases,
    ) -> PortFuture<'_, ModerationResult<Vec<ModerationCase>>>;

    fn apply_action(
        &self,
        request: ApplyModerationAction,
    ) -> PortFuture<'_, ModerationResult<ModerationAction>>;

    fn reverse_action(
        &self,
        request: ReverseModerationAction,
    ) -> PortFuture<'_, ModerationResult<ModerationAction>>;

    fn list_room_actions(
        &self,
        request: ListRoomModeration,
    ) -> PortFuture<'_, ModerationResult<Vec<ModerationAction>>>;

    fn list_audit(
        &self,
        request: ListModerationAudit,
    ) -> PortFuture<'_, ModerationResult<Vec<ModerationAuditEvent>>>;
}

pub struct ModerationDependencies {
    pub repository: Arc<dyn ModerationRepository>,
    pub authority: Arc<dyn ModerationAuthority>,
    pub effects: Arc<dyn ModerationEffectGateway>,
    pub identifiers: Arc<dyn ModerationIdentifierFactory>,
    pub clock: Arc<dyn Clock>,
    pub report_policy: ModerationReportPolicy,
}

pub struct ModerationService {
    repository: Arc<dyn ModerationRepository>,
    authority: Arc<dyn ModerationAuthority>,
    effects: Arc<dyn ModerationEffectGateway>,
    identifiers: Arc<dyn ModerationIdentifierFactory>,
    clock: Arc<dyn Clock>,
    report_policy: ModerationReportPolicy,
}

impl ModerationService {
    pub fn new(dependencies: ModerationDependencies) -> Self {
        Self {
            repository: dependencies.repository,
            authority: dependencies.authority,
            effects: dependencies.effects,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
            report_policy: dependencies.report_policy,
        }
    }

    async fn submit_report_internal(
        &self,
        request: SubmitModerationReport,
    ) -> ModerationResult<ModerationCase> {
        const OPERATION: &str = "moderation.submit_report";
        let now = self.clock.now();
        require_active_actor(&request.actor, now, OPERATION)?;
        let allowed = self
            .authority
            .may_report(
                request.actor.principal_id,
                &request.target,
                request.evidence.room_catalog_id(),
            )
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?;
        if !allowed {
            return Err(failure(OPERATION, ModerationFailureKind::Forbidden));
        }
        let case = ModerationCase::open(
            request.case_id,
            request.actor.principal_id,
            request.target,
            request.reason,
            request.description,
            request.evidence,
            now,
        )
        .map_err(|_| failure(OPERATION, ModerationFailureKind::InvalidRequest))?;
        let correlation_id = self.identifiers.moderation_audit_event_id();
        let audit = audit_event(
            self.identifiers.moderation_audit_event_id(),
            correlation_id,
            &request.actor,
            "moderation.report.created",
            case.target().clone(),
            ModerationAuditOutcome::Allowed,
            Some(case.reason()),
            case.evidence().room_catalog_id(),
            now,
            OPERATION,
        )?;
        match self
            .repository
            .submit_case(&case, &audit, self.report_policy)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
        {
            ModerationReportSubmissionOutcome::Created(case)
            | ModerationReportSubmissionOutcome::Existing(case) => Ok(case),
            ModerationReportSubmissionOutcome::RateLimited { retry_at } => {
                Err(ModerationFailure::rate_limited(OPERATION, retry_at))
            }
        }
    }

    async fn list_my_cases_internal(
        &self,
        request: ListMyModerationCases,
    ) -> ModerationResult<Vec<ModerationCase>> {
        const OPERATION: &str = "moderation.list_my_cases";
        require_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        self.repository
            .list_cases_for_reporter(request.actor.principal_id)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))
    }

    async fn apply_action_internal(
        &self,
        request: ApplyModerationAction,
    ) -> ModerationResult<ModerationAction> {
        const OPERATION: &str = "moderation.apply_action";
        let now = self.clock.now();
        require_recent_actor(&request.actor, now, request.impact_acknowledged, OPERATION)?;
        let context = self
            .authorized_room_context(
                &request.actor,
                request.room_catalog_id,
                &request.target,
                request.kind,
                OPERATION,
            )
            .await?;
        self.require_matching_case(request.case_id, &request.target, OPERATION)
            .await?;
        let mut action = ModerationAction::reserve(
            request.action_id,
            request.case_id,
            request.actor.principal_id,
            request.room_catalog_id,
            request.kind,
            request.target,
            request.reason,
            now,
            request.expires_at,
        )
        .map_err(|_| failure(OPERATION, ModerationFailureKind::InvalidRequest))?;
        let correlation_id = self.identifiers.moderation_audit_event_id();
        let requested_audit = self.action_audit(
            &action,
            "moderation.action.requested",
            ModerationAuditOutcome::Allowed,
            correlation_id,
            now,
            OPERATION,
        )?;
        action = match self
            .repository
            .reserve_action(&action, &requested_audit)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
        {
            ModerationActionReservationOutcome::Reserved(action) => action,
            ModerationActionReservationOutcome::Existing(existing) => return Ok(existing),
        };
        let target = effect_target(&context, action.target().clone());
        match self.effects.apply(&action, &target).await {
            Ok(()) => {
                action
                    .mark_applied()
                    .map_err(|_| failure(OPERATION, ModerationFailureKind::Internal))?;
                let audit = self.action_audit(
                    &action,
                    "moderation.action.applied",
                    ModerationAuditOutcome::Allowed,
                    correlation_id,
                    self.clock.now(),
                    OPERATION,
                )?;
                self.repository
                    .finalize_action(&action, &audit)
                    .await
                    .map_err(|error| repository_failure(OPERATION, &error))
            }
            Err(matrix_failure) => {
                action
                    .mark_failed(matrix_failure_code(matrix_failure))
                    .map_err(|_| failure(OPERATION, ModerationFailureKind::Internal))?;
                let audit = self.action_audit(
                    &action,
                    "moderation.action.failed",
                    ModerationAuditOutcome::Failed,
                    correlation_id,
                    self.clock.now(),
                    OPERATION,
                )?;
                self.repository
                    .finalize_action(&action, &audit)
                    .await
                    .map_err(|error| repository_failure(OPERATION, &error))?;
                Err(failure(
                    OPERATION,
                    ModerationFailureKind::DependencyUnavailable,
                ))
            }
        }
    }

    async fn reverse_action_internal(
        &self,
        request: ReverseModerationAction,
    ) -> ModerationResult<ModerationAction> {
        const OPERATION: &str = "moderation.reverse_action";
        let now = self.clock.now();
        require_recent_actor(&request.actor, now, request.impact_acknowledged, OPERATION)?;
        let mut action = self
            .repository
            .find_action(request.action_id)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
            .ok_or_else(|| failure(OPERATION, ModerationFailureKind::NotFound))?;
        if action.status() == ModerationActionStatus::Reversed {
            return Ok(action);
        }
        let context = self
            .authorized_room_context(
                &request.actor,
                action.room_catalog_id(),
                action.target(),
                action.kind(),
                OPERATION,
            )
            .await?;
        let target = effect_target(&context, action.target().clone());
        if self.effects.reverse(&action, &target).await.is_err() {
            let audit = self.action_audit(
                &action,
                "moderation.action.reverse_failed",
                ModerationAuditOutcome::Failed,
                self.identifiers.moderation_audit_event_id(),
                self.clock.now(),
                OPERATION,
            )?;
            self.repository
                .append_audit(&audit)
                .await
                .map_err(|error| repository_failure(OPERATION, &error))?;
            return Err(failure(
                OPERATION,
                ModerationFailureKind::DependencyUnavailable,
            ));
        }
        action
            .reverse(self.clock.now())
            .map_err(|_| failure(OPERATION, ModerationFailureKind::Conflict))?;
        let correlation_id = self.identifiers.moderation_audit_event_id();
        let audit = self.action_audit(
            &action,
            "moderation.action.reversed",
            ModerationAuditOutcome::Allowed,
            correlation_id,
            self.clock.now(),
            OPERATION,
        )?;
        self.repository
            .finalize_action(&action, &audit)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))
    }

    async fn list_room_actions_internal(
        &self,
        request: ListRoomModeration,
    ) -> ModerationResult<Vec<ModerationAction>> {
        const OPERATION: &str = "moderation.list_room_actions";
        require_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let target = agent_room_domain::moderation::ModerationTarget::new(
            agent_room_domain::moderation::ModerationTargetKind::Room,
            request.room_catalog_id.to_string(),
        )
        .map_err(|_| failure(OPERATION, ModerationFailureKind::Internal))?;
        let context = self
            .authority
            .inspect_room(request.actor.principal_id, request.room_catalog_id, &target)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
            .ok_or_else(|| failure(OPERATION, ModerationFailureKind::NotFound))?;
        if matches!(context.role, ModerationRole::None) {
            return Err(failure(OPERATION, ModerationFailureKind::Forbidden));
        }
        self.repository
            .list_room_actions(request.room_catalog_id)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))
    }

    async fn list_audit_internal(
        &self,
        request: ListModerationAudit,
    ) -> ModerationResult<Vec<ModerationAuditEvent>> {
        const OPERATION: &str = "moderation.list_audit";
        require_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        if request.limit == 0 || request.limit > MAXIMUM_AUDIT_PAGE {
            return Err(failure(OPERATION, ModerationFailureKind::InvalidRequest));
        }
        let role = self
            .authority
            .platform_role(request.actor.principal_id)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?;
        if !role.can_read_audit() {
            return Err(failure(OPERATION, ModerationFailureKind::Forbidden));
        }
        self.repository
            .list_audit(request.room_catalog_id, request.limit)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))
    }

    async fn authorized_room_context(
        &self,
        actor: &crate::authentication::AuthenticatedPrincipal,
        room_catalog_id: agent_room_domain::ids::RoomCatalogId,
        target: &agent_room_domain::moderation::ModerationTarget,
        kind: agent_room_domain::moderation::ModerationActionKind,
        operation: &'static str,
    ) -> ModerationResult<ModerationRoomContext> {
        let context = self
            .authority
            .inspect_room(actor.principal_id, room_catalog_id, target)
            .await
            .map_err(|error| repository_failure(operation, &error))?
            .ok_or_else(|| failure(operation, ModerationFailureKind::NotFound))?;
        if !context.role.allows(kind) {
            return Err(failure(operation, ModerationFailureKind::Forbidden));
        }
        Ok(context)
    }

    async fn require_matching_case(
        &self,
        case_id: Option<agent_room_domain::ids::ModerationCaseId>,
        target: &agent_room_domain::moderation::ModerationTarget,
        operation: &'static str,
    ) -> ModerationResult<()> {
        let Some(case_id) = case_id else {
            return Ok(());
        };
        let case = self
            .repository
            .find_case(case_id)
            .await
            .map_err(|error| repository_failure(operation, &error))?
            .ok_or_else(|| failure(operation, ModerationFailureKind::NotFound))?;
        if case.target() != target {
            return Err(failure(operation, ModerationFailureKind::Conflict));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn action_audit(
        &self,
        action: &ModerationAction,
        audit_action: &'static str,
        outcome: ModerationAuditOutcome,
        correlation_id: AuditEventId,
        occurred_at: agent_room_domain::time::UtcMillis,
        operation: &'static str,
    ) -> ModerationResult<ModerationAuditEvent> {
        ModerationAuditEvent::new(
            self.identifiers.moderation_audit_event_id(),
            occurred_at,
            action.actor_principal_id(),
            audit_action,
            action.target().clone(),
            outcome,
            Some(action.reason()),
            correlation_id,
            Some(action.room_catalog_id()),
        )
        .map_err(|_| failure(operation, ModerationFailureKind::Internal))
    }
}

impl ModerationUseCases for ModerationService {
    fn submit_report(
        &self,
        request: SubmitModerationReport,
    ) -> PortFuture<'_, ModerationResult<ModerationCase>> {
        Box::pin(self.submit_report_internal(request))
    }

    fn list_my_cases(
        &self,
        request: ListMyModerationCases,
    ) -> PortFuture<'_, ModerationResult<Vec<ModerationCase>>> {
        Box::pin(self.list_my_cases_internal(request))
    }

    fn apply_action(
        &self,
        request: ApplyModerationAction,
    ) -> PortFuture<'_, ModerationResult<ModerationAction>> {
        Box::pin(self.apply_action_internal(request))
    }

    fn reverse_action(
        &self,
        request: ReverseModerationAction,
    ) -> PortFuture<'_, ModerationResult<ModerationAction>> {
        Box::pin(self.reverse_action_internal(request))
    }

    fn list_room_actions(
        &self,
        request: ListRoomModeration,
    ) -> PortFuture<'_, ModerationResult<Vec<ModerationAction>>> {
        Box::pin(self.list_room_actions_internal(request))
    }

    fn list_audit(
        &self,
        request: ListModerationAudit,
    ) -> PortFuture<'_, ModerationResult<Vec<ModerationAuditEvent>>> {
        Box::pin(self.list_audit_internal(request))
    }
}

#[allow(clippy::too_many_arguments)]
fn audit_event(
    id: AuditEventId,
    correlation_id: AuditEventId,
    actor: &crate::authentication::AuthenticatedPrincipal,
    action: &'static str,
    target: agent_room_domain::moderation::ModerationTarget,
    outcome: ModerationAuditOutcome,
    reason: Option<agent_room_domain::moderation::ModerationReason>,
    room_catalog_id: Option<agent_room_domain::ids::RoomCatalogId>,
    occurred_at: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> ModerationResult<ModerationAuditEvent> {
    ModerationAuditEvent::new(
        id,
        occurred_at,
        actor.principal_id,
        action,
        target,
        outcome,
        reason,
        correlation_id,
        room_catalog_id,
    )
    .map_err(|_| failure(operation, ModerationFailureKind::Internal))
}

fn effect_target(
    context: &ModerationRoomContext,
    target: agent_room_domain::moderation::ModerationTarget,
) -> ModerationEffectTarget {
    ModerationEffectTarget {
        matrix_room_id: context.matrix_room_id.clone(),
        target,
        target_matrix_user_id: context.target_matrix_user_id.clone(),
    }
}

fn require_active_actor(
    actor: &crate::authentication::AuthenticatedPrincipal,
    now: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> ModerationResult<()> {
    if now < actor.expires_at {
        Ok(())
    } else {
        Err(failure(operation, ModerationFailureKind::Forbidden))
    }
}

fn require_recent_actor(
    actor: &crate::authentication::AuthenticatedPrincipal,
    now: agent_room_domain::time::UtcMillis,
    impact_acknowledged: bool,
    operation: &'static str,
) -> ModerationResult<()> {
    require_active_actor(actor, now, operation)?;
    if actor.recently_authenticated && impact_acknowledged {
        Ok(())
    } else {
        Err(failure(operation, ModerationFailureKind::Forbidden))
    }
}

fn repository_failure(operation: &'static str, error: &RepositoryError) -> ModerationFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Conflict => ModerationFailureKind::Conflict,
        RepositoryErrorKind::Forbidden => ModerationFailureKind::Forbidden,
        RepositoryErrorKind::NotFound => ModerationFailureKind::NotFound,
        RepositoryErrorKind::Constraint | RepositoryErrorKind::CorruptData => {
            ModerationFailureKind::Internal
        }
        RepositoryErrorKind::Unavailable => ModerationFailureKind::DependencyUnavailable,
    };
    failure(operation, kind)
}

fn matrix_failure_code(failure: MatrixFailure) -> &'static str {
    match failure.kind() {
        crate::ports::MatrixFailureKind::InvalidConfiguration => "matrix.invalid_configuration",
        crate::ports::MatrixFailureKind::Unauthenticated => "matrix.unauthenticated",
        crate::ports::MatrixFailureKind::AuthenticationRejected => "matrix.auth_rejected",
        crate::ports::MatrixFailureKind::Forbidden => "matrix.forbidden",
        crate::ports::MatrixFailureKind::NotFound => "matrix.not_found",
        crate::ports::MatrixFailureKind::Conflict => "matrix.conflict",
        crate::ports::MatrixFailureKind::RateLimited => "matrix.rate_limited",
        crate::ports::MatrixFailureKind::Timeout => "matrix.timeout",
        crate::ports::MatrixFailureKind::DependencyUnavailable => "matrix.unavailable",
        crate::ports::MatrixFailureKind::InvalidResponse => "matrix.invalid_response",
        crate::ports::MatrixFailureKind::UnknownCommit => "matrix.unknown_commit",
        crate::ports::MatrixFailureKind::StaleSyncToken => "matrix.stale_sync_token",
        crate::ports::MatrixFailureKind::UnsupportedVersion => "matrix.unsupported_version",
    }
}

const fn failure(operation: &'static str, kind: ModerationFailureKind) -> ModerationFailure {
    ModerationFailure::new(operation, kind)
}
