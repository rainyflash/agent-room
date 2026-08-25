use std::sync::Arc;

use agent_room_domain::{
    ids::PrincipalId,
    policy::{
        AutomationGrant, AutomationGrantAttempt, AutomationGrantDecision, AutomationGrantFields,
        AutomationMessageKind, AutomationRiskScanOutcome,
    },
};

use crate::{
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        AutomationConsumptionOutcome, AutomationConsumptionRequest, AutomationDecisionRecord,
        AutomationGrantRecord, AutomationGrantRepository, AutomationGrantRevocationOutcome,
        AutomationScopeAuthority, AutomationScopeAuthorityRequest, AutomationSendAuthorityRequest,
        Clock, MatrixRoomAuthorityGateway, PortFuture,
    },
};

use super::{
    AuthorizeAutomationSend, AutomationAuthorizationOutcome, AutomationAuthorizationReceipt,
    AutomationFailure, AutomationFailureKind, AutomationResult, AutomationSendDenial,
    CreateAutomationGrant, ListAutomationGrants, RevokeAutomationGrant,
    models::AutomationGrantList,
};

pub trait AutomationUseCases: Send + Sync {
    fn create(
        &self,
        request: CreateAutomationGrant,
    ) -> PortFuture<'_, AutomationResult<AutomationGrantRecord>>;

    fn list(
        &self,
        request: ListAutomationGrants,
    ) -> PortFuture<'_, AutomationResult<AutomationGrantList>>;

    fn revoke(
        &self,
        request: RevokeAutomationGrant,
    ) -> PortFuture<'_, AutomationResult<AutomationGrantRecord>>;

    fn authorize_send(
        &self,
        request: AuthorizeAutomationSend,
    ) -> PortFuture<'_, AutomationResult<AutomationAuthorizationOutcome>>;
}

pub struct AutomationDependencies {
    pub grants: Arc<dyn AutomationGrantRepository>,
    pub authority: Arc<dyn AutomationScopeAuthority>,
    pub matrix_authority: Arc<dyn MatrixRoomAuthorityGateway>,
    pub clock: Arc<dyn Clock>,
}

pub struct AutomationService {
    grants: Arc<dyn AutomationGrantRepository>,
    authority: Arc<dyn AutomationScopeAuthority>,
    matrix_authority: Arc<dyn MatrixRoomAuthorityGateway>,
    clock: Arc<dyn Clock>,
}

impl AutomationService {
    pub fn new(dependencies: AutomationDependencies) -> Self {
        Self {
            grants: dependencies.grants,
            authority: dependencies.authority,
            matrix_authority: dependencies.matrix_authority,
            clock: dependencies.clock,
        }
    }

    async fn create_internal(
        &self,
        request: CreateAutomationGrant,
    ) -> AutomationResult<AutomationGrantRecord> {
        const OPERATION: &str = "automation.create";
        let now = self.clock.now();
        require_recent_actor(&request.actor, now, request.impact_acknowledged, OPERATION)?;
        let authority = AutomationScopeAuthorityRequest {
            principal_id: request.actor.principal_id,
            agent_id: request.scope.agent_id(),
            agent_instance_id: request.scope.agent_instance_id(),
            room_catalog_id: request.scope.room_catalog_id(),
        };
        let allowed = self
            .authority
            .may_create(&authority)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?;
        if !allowed {
            return Err(failure(OPERATION, AutomationFailureKind::Forbidden));
        }
        let grant = AutomationGrant::issue(AutomationGrantFields {
            id: request.grant_id,
            grantor_id: request.actor.principal_id,
            scope: request.scope,
            limits: request.limits,
            created_at: now,
        })
        .map_err(|_| failure(OPERATION, AutomationFailureKind::InvalidRequest))?;
        self.grants
            .create(&grant)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))
    }

    async fn list_internal(
        &self,
        request: ListAutomationGrants,
    ) -> AutomationResult<AutomationGrantList> {
        const OPERATION: &str = "automation.list";
        let now = self.clock.now();
        require_active_actor(&request.actor, now, OPERATION)?;
        self.grants
            .list_for_principal(request.actor.principal_id, now)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))
    }

    async fn revoke_internal(
        &self,
        request: RevokeAutomationGrant,
    ) -> AutomationResult<AutomationGrantRecord> {
        const OPERATION: &str = "automation.revoke";
        let now = self.clock.now();
        require_recent_actor(&request.actor, now, true, OPERATION)?;
        match self
            .grants
            .revoke(request.actor.principal_id, request.grant_id, now)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
        {
            AutomationGrantRevocationOutcome::Revoked(record)
            | AutomationGrantRevocationOutcome::AlreadyRevoked(record) => Ok(record),
            AutomationGrantRevocationOutcome::NotFound => {
                Err(failure(OPERATION, AutomationFailureKind::NotFound))
            }
        }
    }

    async fn authorize_send_internal(
        &self,
        request: AuthorizeAutomationSend,
    ) -> AutomationResult<AutomationAuthorizationOutcome> {
        const OPERATION: &str = "automation.authorize_send";
        let now = self.clock.now();
        let preparation = self.load_and_precheck(&request, now).await?;
        let SendPreparation::Ready(context) = preparation else {
            return Ok(preparation.denied_outcome());
        };
        let preparation = self
            .apply_current_authority(&request, *context, now)
            .await?;
        let SendPreparation::Ready(context) = preparation else {
            return Ok(preparation.denied_outcome());
        };
        let consumption = AutomationConsumptionRequest {
            grant_id: request.grant_id,
            submission_id: request.submission_id,
            attempt: context.attempt,
        };
        match self
            .grants
            .consume(&consumption)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
        {
            AutomationConsumptionOutcome::Consumed { reused, .. } => Ok(
                AutomationAuthorizationOutcome::Authorized(AutomationAuthorizationReceipt {
                    grant_id: request.grant_id,
                    submission_id: request.submission_id,
                    reused,
                }),
            ),
            AutomationConsumptionOutcome::Denied(reason) => Ok(
                AutomationAuthorizationOutcome::Denied(AutomationSendDenial::Grant(reason)),
            ),
            AutomationConsumptionOutcome::NotFound => Ok(AutomationAuthorizationOutcome::Denied(
                AutomationSendDenial::GrantNotFound,
            )),
        }
    }

    async fn load_and_precheck(
        &self,
        request: &AuthorizeAutomationSend,
        now: agent_room_domain::time::UtcMillis,
    ) -> AutomationResult<SendPreparation> {
        const OPERATION: &str = "automation.authorize_send";
        let principal_id = request.actor.account.principal.id();
        let Some(record) = self
            .grants
            .find(request.grant_id, now)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
        else {
            return Ok(SendPreparation::Denied(AutomationSendDenial::GrantNotFound));
        };
        if principal_id != record.grant.grantor_id() {
            return self
                .deny_preparation(
                    request,
                    principal_id,
                    AutomationSendDenial::ActorMismatch,
                    now,
                )
                .await;
        }

        let message_kind = if request.is_reply {
            AutomationMessageKind::Reply
        } else {
            AutomationMessageKind::RoomMessage
        };
        let preliminary = attempt(
            request,
            message_kind,
            false,
            AutomationRiskScanOutcome::Passed,
            now,
        );
        if let AutomationGrantDecision::Denied(reason) =
            record.grant.evaluate(&preliminary, record.usage)
        {
            return self
                .deny_preparation(
                    request,
                    principal_id,
                    AutomationSendDenial::Grant(reason),
                    now,
                )
                .await;
        }
        Ok(SendPreparation::Ready(Box::new(PreparedSend {
            principal_id,
            record,
            message_kind,
            attempt: preliminary,
        })))
    }

    async fn apply_current_authority(
        &self,
        request: &AuthorizeAutomationSend,
        mut context: PreparedSend,
        now: agent_room_domain::time::UtcMillis,
    ) -> AutomationResult<SendPreparation> {
        const OPERATION: &str = "automation.authorize_send";
        let authority_request = AutomationSendAuthorityRequest {
            principal_id: context.principal_id,
            device_id: request.actor.device_id,
            agent_id: request.agent_id,
            agent_instance_id: request.agent_instance_id,
            room_catalog_id: request.room_catalog_id,
            matrix_room_id: request.matrix_room_id.clone(),
        };
        let Some(authority) = self
            .authority
            .inspect_send(&authority_request)
            .await
            .map_err(|error| repository_failure(OPERATION, &error))?
        else {
            return self
                .deny_preparation(
                    request,
                    context.principal_id,
                    AutomationSendDenial::AuthorityChanged,
                    now,
                )
                .await;
        };

        let scoped = attempt(
            request,
            context.message_kind,
            authority.contains_unknown_recipients,
            AutomationRiskScanOutcome::Passed,
            now,
        );
        if let AutomationGrantDecision::Denied(reason) =
            context.record.grant.evaluate(&scoped, context.record.usage)
        {
            return self
                .deny_preparation(
                    request,
                    context.principal_id,
                    AutomationSendDenial::Grant(reason),
                    now,
                )
                .await;
        }

        let Ok(matrix) = self
            .matrix_authority
            .inspect_room_authority(&request.matrix_room_id, &authority.agent_matrix_user_id)
            .await
        else {
            self.record_denial(
                request,
                context.principal_id,
                "automation.matrix_authority_unavailable",
                now,
            )
            .await?;
            return Err(failure(
                OPERATION,
                AutomationFailureKind::DependencyUnavailable,
            ));
        };
        if !matrix.is_joined() || !matrix.power_level().is_at_least(0) {
            return self
                .deny_preparation(
                    request,
                    context.principal_id,
                    AutomationSendDenial::MatrixPermissionDenied,
                    now,
                )
                .await;
        }

        let final_attempt = attempt(
            request,
            context.message_kind,
            authority.contains_unknown_recipients,
            request.risk_scan,
            now,
        );
        if let AutomationGrantDecision::Denied(reason) = context
            .record
            .grant
            .evaluate(&final_attempt, context.record.usage)
        {
            return self
                .deny_preparation(
                    request,
                    context.principal_id,
                    AutomationSendDenial::Grant(reason),
                    now,
                )
                .await;
        }
        context.attempt = final_attempt;
        Ok(SendPreparation::Ready(Box::new(context)))
    }

    async fn deny_preparation(
        &self,
        request: &AuthorizeAutomationSend,
        principal_id: PrincipalId,
        reason: AutomationSendDenial,
        now: agent_room_domain::time::UtcMillis,
    ) -> AutomationResult<SendPreparation> {
        self.record_denial(request, principal_id, reason.as_str(), now)
            .await?;
        Ok(SendPreparation::Denied(reason))
    }

    async fn record_denial(
        &self,
        request: &AuthorizeAutomationSend,
        principal_id: PrincipalId,
        decision_code: &'static str,
        now: agent_room_domain::time::UtcMillis,
    ) -> AutomationResult<()> {
        self.grants
            .record_decision(&AutomationDecisionRecord {
                grant_id: request.grant_id,
                submission_id: request.submission_id,
                principal_id,
                agent_id: request.agent_id,
                agent_instance_id: request.agent_instance_id,
                room_catalog_id: request.room_catalog_id,
                matrix_room_id: request.matrix_room_id.clone(),
                decision_code,
                decided_at: now,
            })
            .await
            .map_err(|error| repository_failure("automation.record_decision", &error))
    }
}

struct PreparedSend {
    principal_id: PrincipalId,
    record: AutomationGrantRecord,
    message_kind: AutomationMessageKind,
    attempt: AutomationGrantAttempt,
}

enum SendPreparation {
    Ready(Box<PreparedSend>),
    Denied(AutomationSendDenial),
}

impl SendPreparation {
    const fn denied_outcome(&self) -> AutomationAuthorizationOutcome {
        match self {
            Self::Denied(reason) => AutomationAuthorizationOutcome::Denied(*reason),
            Self::Ready(_) => unreachable!(),
        }
    }
}

impl AutomationUseCases for AutomationService {
    fn create(
        &self,
        request: CreateAutomationGrant,
    ) -> PortFuture<'_, AutomationResult<AutomationGrantRecord>> {
        Box::pin(self.create_internal(request))
    }

    fn list(
        &self,
        request: ListAutomationGrants,
    ) -> PortFuture<'_, AutomationResult<AutomationGrantList>> {
        Box::pin(self.list_internal(request))
    }

    fn revoke(
        &self,
        request: RevokeAutomationGrant,
    ) -> PortFuture<'_, AutomationResult<AutomationGrantRecord>> {
        Box::pin(self.revoke_internal(request))
    }

    fn authorize_send(
        &self,
        request: AuthorizeAutomationSend,
    ) -> PortFuture<'_, AutomationResult<AutomationAuthorizationOutcome>> {
        Box::pin(self.authorize_send_internal(request))
    }
}

fn attempt(
    request: &AuthorizeAutomationSend,
    message_kind: AutomationMessageKind,
    contains_unknown_recipients: bool,
    risk_scan: AutomationRiskScanOutcome,
    now: agent_room_domain::time::UtcMillis,
) -> AutomationGrantAttempt {
    AutomationGrantAttempt {
        agent_id: request.agent_id,
        agent_instance_id: Some(request.agent_instance_id),
        room_catalog_id: request.room_catalog_id,
        message_kind,
        contains_unknown_recipients,
        risk_scan,
        now,
    }
}

fn require_active_actor(
    actor: &crate::authentication::AuthenticatedPrincipal,
    now: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> AutomationResult<()> {
    if now >= actor.expires_at {
        return Err(failure(operation, AutomationFailureKind::Forbidden));
    }
    Ok(())
}

fn require_recent_actor(
    actor: &crate::authentication::AuthenticatedPrincipal,
    now: agent_room_domain::time::UtcMillis,
    impact_acknowledged: bool,
    operation: &'static str,
) -> AutomationResult<()> {
    require_active_actor(actor, now, operation)?;
    if !actor.recently_authenticated || !impact_acknowledged {
        return Err(failure(operation, AutomationFailureKind::Forbidden));
    }
    Ok(())
}

fn repository_failure(operation: &'static str, error: &RepositoryError) -> AutomationFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Conflict => AutomationFailureKind::Conflict,
        RepositoryErrorKind::Forbidden => AutomationFailureKind::Forbidden,
        RepositoryErrorKind::NotFound => AutomationFailureKind::NotFound,
        RepositoryErrorKind::Constraint | RepositoryErrorKind::CorruptData => {
            AutomationFailureKind::Internal
        }
        RepositoryErrorKind::Unavailable => AutomationFailureKind::DependencyUnavailable,
    };
    failure(operation, kind)
}

const fn failure(operation: &'static str, kind: AutomationFailureKind) -> AutomationFailure {
    AutomationFailure::new(operation, kind)
}
