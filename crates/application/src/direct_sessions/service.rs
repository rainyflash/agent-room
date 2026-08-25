use std::sync::Arc;

use agent_room_domain::{
    direct_sessions::{DirectContactPolicy, DirectSession, DirectSessionLifecycle},
    ids::{AgentId, PrincipalId, RoomCatalogId},
    rooms::{
        MatrixRoomReference, RoomCatalog, RoomCatalogFields, RoomCatalogKind, RoomCatalogStatus,
        RoomCatalogVisibility,
    },
};

use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryErrorKind,
    ports::{
        Clock, DirectAgentProfile, DirectMatrixRoomCreation, DirectSessionAgentDirectory,
        DirectSessionMatrixProvisioner, DirectSessionRecord, DirectSessionStore, IdentifierFactory,
        MatrixCreateRoom, MatrixRoomAliasLocalpart, MatrixRoomPreset, MatrixRoomVisibility,
        MatrixUserId, PortFuture,
    },
};

use super::{
    DirectContactView, DirectSessionFailureKind, DirectSessionFailureStage, DirectSessionResult,
    DirectSessionView, InspectDirectSession, ListDirectSessions, OpenDirectSession,
    SetDirectAgentBlock,
    failure::{domain, failure, matrix, repository},
};

pub trait DirectSessionUseCases: Send + Sync {
    fn open(
        &self,
        request: OpenDirectSession,
    ) -> PortFuture<'_, DirectSessionResult<DirectSessionView>>;

    fn inspect(
        &self,
        request: InspectDirectSession,
    ) -> PortFuture<'_, DirectSessionResult<DirectSessionView>>;

    fn list(
        &self,
        request: ListDirectSessions,
    ) -> PortFuture<'_, DirectSessionResult<Vec<DirectSessionView>>>;

    fn set_block(
        &self,
        request: SetDirectAgentBlock,
    ) -> PortFuture<'_, DirectSessionResult<DirectContactView>>;
}

pub struct DirectSessionDependencies {
    pub store: Arc<dyn DirectSessionStore>,
    pub agents: Arc<dyn DirectSessionAgentDirectory>,
    pub matrix: Arc<dyn DirectSessionMatrixProvisioner>,
    pub identifiers: Arc<dyn IdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

pub struct DirectSessionService {
    store: Arc<dyn DirectSessionStore>,
    agents: Arc<dyn DirectSessionAgentDirectory>,
    matrix: Arc<dyn DirectSessionMatrixProvisioner>,
    identifiers: Arc<dyn IdentifierFactory>,
    clock: Arc<dyn Clock>,
}

impl DirectSessionService {
    pub fn new(dependencies: DirectSessionDependencies) -> Self {
        Self {
            store: dependencies.store,
            agents: dependencies.agents,
            matrix: dependencies.matrix,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
        }
    }

    async fn open_internal(
        &self,
        request: OpenDirectSession,
    ) -> DirectSessionResult<DirectSessionView> {
        const OPERATION: &str = "direct_session.open";
        let actor_matrix = active_actor_matrix_user(&request.actor, self.clock.now(), OPERATION)?;
        let existing = self
            .store
            .find_by_participants(request.actor.principal_id, request.target_agent_id)
            .await
            .map_err(|error| {
                repository(OPERATION, DirectSessionFailureStage::Persistence, &error)
            })?;
        let target = match existing.as_ref() {
            Some(_) => {
                self.load_known_target(
                    request.actor.principal_id,
                    request.target_agent_id,
                    OPERATION,
                )
                .await?
            }
            None => {
                self.load_contactable_target(
                    request.actor.principal_id,
                    request.target_agent_id,
                    OPERATION,
                )
                .await?
            }
        };
        let contact_policy = self
            .contact_policy(
                request.actor.principal_id,
                request.target_agent_id,
                OPERATION,
            )
            .await?;
        if !contact_policy.delivery_allowed() {
            return Err(failure(
                OPERATION,
                DirectSessionFailureStage::Validation,
                DirectSessionFailureKind::Blocked,
            ));
        }
        let record = match existing {
            Some(record) => record,
            None => self.reserve(&request.actor, &target, OPERATION).await?,
        };
        let record = self
            .ensure_active(record, &actor_matrix, &target, OPERATION)
            .await?;
        Ok(DirectSessionView {
            record,
            target,
            contact_policy,
        })
    }

    async fn inspect_internal(
        &self,
        request: InspectDirectSession,
    ) -> DirectSessionResult<DirectSessionView> {
        const OPERATION: &str = "direct_session.inspect";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let record = self
            .store
            .find_by_catalog(request.catalog_id)
            .await
            .map_err(|error| repository(OPERATION, DirectSessionFailureStage::Persistence, &error))?
            .filter(|record| record.session().principal_id() == request.actor.principal_id)
            .filter(|record| record.session().is_active())
            .ok_or_else(|| {
                failure(
                    OPERATION,
                    DirectSessionFailureStage::Persistence,
                    DirectSessionFailureKind::NotFound,
                )
            })?;
        self.view_for_record(record, request.actor.principal_id, OPERATION)
            .await
    }

    async fn list_internal(
        &self,
        request: ListDirectSessions,
    ) -> DirectSessionResult<Vec<DirectSessionView>> {
        const OPERATION: &str = "direct_session.list";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let records = self
            .store
            .list_for_principal(request.actor.principal_id)
            .await
            .map_err(|error| {
                repository(OPERATION, DirectSessionFailureStage::Persistence, &error)
            })?;
        let mut views = Vec::with_capacity(records.len());
        for record in records {
            views.push(
                self.view_for_record(record, request.actor.principal_id, OPERATION)
                    .await?,
            );
        }
        Ok(views)
    }

    async fn set_block_internal(
        &self,
        request: SetDirectAgentBlock,
    ) -> DirectSessionResult<DirectContactView> {
        const OPERATION: &str = "direct_session.set_block";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let target = self
            .load_known_target(
                request.actor.principal_id,
                request.target_agent_id,
                OPERATION,
            )
            .await?;
        let contact_policy = self
            .store
            .set_principal_block(
                request.actor.principal_id,
                request.target_agent_id,
                request.blocked,
                self.clock.now(),
            )
            .await
            .map_err(|error| {
                repository(OPERATION, DirectSessionFailureStage::Persistence, &error)
            })?;
        Ok(DirectContactView {
            target,
            contact_policy,
        })
    }

    async fn reserve(
        &self,
        actor: &AuthenticatedPrincipal,
        target: &DirectAgentProfile,
        operation: &'static str,
    ) -> DirectSessionResult<DirectSessionRecord> {
        let catalog_id = self.identifiers.room_catalog_id();
        let catalog = RoomCatalog::new(
            catalog_id,
            RoomCatalogFields {
                kind: RoomCatalogKind::Direct,
                slug: None,
                name: target.display_name.clone(),
                description: String::new(),
                language: None,
                matrix_space_id: None,
                owner_principal_id: Some(actor.principal_id),
                visibility: RoomCatalogVisibility::Private,
                retention_days: None,
                status: RoomCatalogStatus::Frozen,
            },
        )
        .map_err(|error| domain(operation, &error))?;
        let session = DirectSession::reserve(catalog_id, actor.principal_id, target.agent_id);
        let record = DirectSessionRecord::new(catalog, None, session)
            .map_err(|error| domain(operation, &error))?;
        self.store
            .reserve(&record, self.clock.now())
            .await
            .map_err(|error| repository(operation, DirectSessionFailureStage::Persistence, &error))
    }

    async fn ensure_active(
        &self,
        record: DirectSessionRecord,
        actor_matrix: &MatrixUserId,
        target: &DirectAgentProfile,
        operation: &'static str,
    ) -> DirectSessionResult<DirectSessionRecord> {
        match record.session().lifecycle() {
            DirectSessionLifecycle::Active => return Ok(record),
            DirectSessionLifecycle::Failed => {
                return Err(failure(
                    operation,
                    DirectSessionFailureStage::Persistence,
                    DirectSessionFailureKind::Conflict,
                ));
            }
            DirectSessionLifecycle::Provisioning => {}
        }
        let alias = direct_room_alias(record.session().catalog_id(), operation)?;
        let matrix_request = MatrixCreateRoom::new(
            None,
            None,
            MatrixRoomVisibility::Private,
            MatrixRoomPreset::TrustedPrivateChat,
            true,
            vec![actor_matrix.clone()],
        )
        .map_err(|error| domain(operation, &error))?
        .with_alias_localpart(alias.clone());
        let creation = DirectMatrixRoomCreation::new(
            matrix_request,
            alias,
            target.matrix_user_id.clone(),
            actor_matrix.clone(),
        )
        .map_err(|error| domain(operation, &error))?;
        let matrix_room_id = self
            .matrix
            .create(&creation)
            .await
            .map_err(|error| matrix(operation, error))?;
        let matrix_reference = MatrixRoomReference::new(matrix_room_id.as_str().to_owned())
            .map_err(|error| domain(operation, &error))?;
        let expected = record.session().version();
        let active = record
            .activate(self.identifiers.room_instance_id(), matrix_reference)
            .map_err(|error| domain(operation, &error))?;
        match self
            .store
            .activate(&active, expected, self.clock.now())
            .await
        {
            Ok(record) => Ok(record),
            Err(error) if error.kind() == RepositoryErrorKind::Conflict => self
                .store
                .find_by_participants(
                    active.session().principal_id(),
                    active.session().target_agent_id(),
                )
                .await
                .map_err(|reload| {
                    repository(operation, DirectSessionFailureStage::Persistence, &reload)
                })?
                .filter(|record| record.session().is_active())
                .ok_or_else(|| {
                    failure(
                        operation,
                        DirectSessionFailureStage::Persistence,
                        DirectSessionFailureKind::Conflict,
                    )
                }),
            Err(error) => Err(repository(
                operation,
                DirectSessionFailureStage::Persistence,
                &error,
            )),
        }
    }

    async fn view_for_record(
        &self,
        record: DirectSessionRecord,
        actor_principal_id: PrincipalId,
        operation: &'static str,
    ) -> DirectSessionResult<DirectSessionView> {
        let target = self
            .load_known_target(
                actor_principal_id,
                record.session().target_agent_id(),
                operation,
            )
            .await?;
        let contact_policy = self
            .contact_policy(actor_principal_id, target.agent_id, operation)
            .await?;
        Ok(DirectSessionView {
            record,
            target,
            contact_policy,
        })
    }

    async fn load_contactable_target(
        &self,
        actor_principal_id: PrincipalId,
        target_agent_id: AgentId,
        operation: &'static str,
    ) -> DirectSessionResult<DirectAgentProfile> {
        self.agents
            .find_contactable(actor_principal_id, target_agent_id)
            .await
            .map_err(|error| repository(operation, DirectSessionFailureStage::Directory, &error))?
            .ok_or_else(|| {
                failure(
                    operation,
                    DirectSessionFailureStage::Directory,
                    DirectSessionFailureKind::NotFound,
                )
            })
    }

    async fn load_known_target(
        &self,
        actor_principal_id: PrincipalId,
        target_agent_id: AgentId,
        operation: &'static str,
    ) -> DirectSessionResult<DirectAgentProfile> {
        self.agents
            .find_known_contact(actor_principal_id, target_agent_id)
            .await
            .map_err(|error| repository(operation, DirectSessionFailureStage::Directory, &error))?
            .ok_or_else(|| {
                failure(
                    operation,
                    DirectSessionFailureStage::Directory,
                    DirectSessionFailureKind::NotFound,
                )
            })
    }

    async fn contact_policy(
        &self,
        principal_id: PrincipalId,
        agent_id: AgentId,
        operation: &'static str,
    ) -> DirectSessionResult<DirectContactPolicy> {
        self.store
            .contact_policy(principal_id, agent_id)
            .await
            .map_err(|error| repository(operation, DirectSessionFailureStage::Persistence, &error))
    }
}

impl DirectSessionUseCases for DirectSessionService {
    fn open(
        &self,
        request: OpenDirectSession,
    ) -> PortFuture<'_, DirectSessionResult<DirectSessionView>> {
        Box::pin(self.open_internal(request))
    }

    fn inspect(
        &self,
        request: InspectDirectSession,
    ) -> PortFuture<'_, DirectSessionResult<DirectSessionView>> {
        Box::pin(self.inspect_internal(request))
    }

    fn list(
        &self,
        request: ListDirectSessions,
    ) -> PortFuture<'_, DirectSessionResult<Vec<DirectSessionView>>> {
        Box::pin(self.list_internal(request))
    }

    fn set_block(
        &self,
        request: SetDirectAgentBlock,
    ) -> PortFuture<'_, DirectSessionResult<DirectContactView>> {
        Box::pin(self.set_block_internal(request))
    }
}

fn ensure_active_actor(
    actor: &AuthenticatedPrincipal,
    now: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> DirectSessionResult<()> {
    if now < actor.expires_at {
        Ok(())
    } else {
        Err(failure(
            operation,
            DirectSessionFailureStage::Validation,
            DirectSessionFailureKind::Forbidden,
        ))
    }
}

fn active_actor_matrix_user(
    actor: &AuthenticatedPrincipal,
    now: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> DirectSessionResult<MatrixUserId> {
    ensure_active_actor(actor, now, operation)?;
    MatrixUserId::new(actor.matrix_user_id.clone()).map_err(|_| {
        failure(
            operation,
            DirectSessionFailureStage::Validation,
            DirectSessionFailureKind::Internal,
        )
    })
}

fn direct_room_alias(
    catalog_id: RoomCatalogId,
    operation: &'static str,
) -> DirectSessionResult<MatrixRoomAliasLocalpart> {
    MatrixRoomAliasLocalpart::new(format!(
        "agent-room-direct-{}",
        catalog_id.as_uuid().simple()
    ))
    .map_err(|_| {
        failure(
            operation,
            DirectSessionFailureStage::Validation,
            DirectSessionFailureKind::Internal,
        )
    })
}
