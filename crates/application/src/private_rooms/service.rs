use std::{collections::BTreeSet, sync::Arc};

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::{PrivateRoom, PrivateRoomMembershipStatus},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
    },
};

use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryErrorKind,
    ports::{
        Clock, IdentifierFactory, MatrixCreateRoom, MatrixRoomAliasLocalpart, MatrixRoomId,
        MatrixRoomPowerProfile, MatrixRoomPreset, MatrixRoomVisibility, MatrixUserId, PortFuture,
        PrivateMatrixMembership, PrivateMatrixRoomCreation, PrivateMatrixSpeakingAssignment,
        PrivateRoomMatrixGateway, PrivateRoomMatrixProvisioner, PrivateRoomPrincipalDirectory,
        PrivateRoomSnapshot, PrivateRoomStore,
    },
};

use super::{
    ArchivePrivateRoom, ChangePrivateRoomPermissions, CreatePrivateRoom, GovernPrivateRoomMember,
    InspectPrivateRoom, InvitePrivateRoomMember, PrivateRoomFailureKind, PrivateRoomFailureStage,
    PrivateRoomMembershipAction, PrivateRoomResult, TransferPrivateRoomOwnership,
    failure::{domain, failure, matrix, repository},
};

pub trait PrivateRoomUseCases: Send + Sync {
    fn create(
        &self,
        request: CreatePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn inspect(
        &self,
        request: InspectPrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn invite(
        &self,
        request: InvitePrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn accept(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn decline(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn leave(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn remove(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn ban(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn update_permissions(
        &self,
        request: ChangePrivateRoomPermissions,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn transfer_ownership(
        &self,
        request: TransferPrivateRoomOwnership,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn archive(
        &self,
        request: ArchivePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;
}

pub struct PrivateRoomDependencies {
    pub store: Arc<dyn PrivateRoomStore>,
    pub matrix_provisioner: Arc<dyn PrivateRoomMatrixProvisioner>,
    pub matrix: Arc<dyn PrivateRoomMatrixGateway>,
    pub principals: Arc<dyn PrivateRoomPrincipalDirectory>,
    pub identifiers: Arc<dyn IdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

pub struct PrivateRoomService {
    store: Arc<dyn PrivateRoomStore>,
    matrix_provisioner: Arc<dyn PrivateRoomMatrixProvisioner>,
    matrix: Arc<dyn PrivateRoomMatrixGateway>,
    principals: Arc<dyn PrivateRoomPrincipalDirectory>,
    identifiers: Arc<dyn IdentifierFactory>,
    clock: Arc<dyn Clock>,
}

impl PrivateRoomService {
    pub fn new(dependencies: PrivateRoomDependencies) -> Self {
        Self {
            store: dependencies.store,
            matrix_provisioner: dependencies.matrix_provisioner,
            matrix: dependencies.matrix,
            principals: dependencies.principals,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
        }
    }

    async fn create_internal(
        &self,
        request: CreatePrivateRoom,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.create";
        let owner_matrix_user =
            active_actor_matrix_user(&request.actor, self.clock.now(), OPERATION)?;
        if let Some(existing) = self.find(request.catalog_id, OPERATION).await? {
            return ensure_same_creation(&request, existing, OPERATION);
        }

        let prepared = self
            .prepare_creation(&request, owner_matrix_user, OPERATION)
            .await?;
        let matrix_room_id = self
            .matrix_provisioner
            .create(&prepared.matrix)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.matrix
            .set_speaking_batch(&matrix_room_id, &prepared.speaking_assignments)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        let snapshot = self.build_snapshot(&request, prepared.room, &matrix_room_id, OPERATION)?;
        let now = self.clock.now();
        match self.store.create(&snapshot, now).await {
            Ok(()) => Ok(snapshot),
            Err(error) if error.kind() == RepositoryErrorKind::Conflict => {
                let existing =
                    self.find(request.catalog_id, OPERATION)
                        .await?
                        .ok_or_else(|| {
                            failure(
                                OPERATION,
                                PrivateRoomFailureStage::Persistence,
                                PrivateRoomFailureKind::Conflict,
                            )
                        })?;
                ensure_same_creation(&request, existing, OPERATION)
            }
            Err(error) => Err(repository(
                OPERATION,
                PrivateRoomFailureStage::Persistence,
                &error,
            )),
        }
    }

    async fn inspect_internal(
        &self,
        request: InspectPrivateRoom,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.inspect";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let visible = snapshot
            .room()
            .member(request.actor.principal_id)
            .is_some_and(|member| {
                matches!(
                    member.status(),
                    PrivateRoomMembershipStatus::Invited | PrivateRoomMembershipStatus::Joined
                )
            });
        if !visible {
            return Err(failure(
                OPERATION,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::NotFound,
            ));
        }
        Ok(snapshot)
    }

    async fn invite_internal(
        &self,
        request: InvitePrivateRoomMember,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.invite";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .invite(
                request.actor.principal_id,
                request.target_principal_id,
                request.permissions,
            )
            .map_err(|error| domain(OPERATION, &error))?;
        let target = self
            .principal_matrix_user(request.target_principal_id, OPERATION)
            .await?;
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;
        self.matrix
            .invite(&matrix_room, &target)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        replace_room(snapshot, room, OPERATION)
    }

    async fn accept_internal(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.accept";
        let actor_matrix = active_actor_matrix_user(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .accept_invitation(request.actor.principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        self.require_matrix_membership(
            &snapshot,
            &actor_matrix,
            MatrixMembershipRequirement::Joined,
            OPERATION,
        )
        .await?;
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        replace_room(snapshot, room, OPERATION)
    }

    async fn decline_internal(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.decline";
        let actor_matrix = active_actor_matrix_user(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .decline_invitation(request.actor.principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        self.require_matrix_membership(
            &snapshot,
            &actor_matrix,
            MatrixMembershipRequirement::AbsentOrLeft,
            OPERATION,
        )
        .await?;
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        replace_room(snapshot, room, OPERATION)
    }

    async fn leave_internal(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.leave";
        let actor_matrix = active_actor_matrix_user(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .leave(request.actor.principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        self.require_matrix_membership(
            &snapshot,
            &actor_matrix,
            MatrixMembershipRequirement::AbsentOrLeft,
            OPERATION,
        )
        .await?;
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        replace_room(snapshot, room, OPERATION)
    }

    async fn remove_internal(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.remove";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .remove_member(request.actor.principal_id, request.target_principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        let target = self
            .principal_matrix_user(request.target_principal_id, OPERATION)
            .await?;
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;
        self.matrix
            .kick(&matrix_room, &target)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        replace_room(snapshot, room, OPERATION)
    }

    async fn ban_internal(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.ban";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .ban_member(request.actor.principal_id, request.target_principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        let target = self
            .principal_matrix_user(request.target_principal_id, OPERATION)
            .await?;
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;
        self.matrix
            .ban(&matrix_room, &target)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        replace_room(snapshot, room, OPERATION)
    }

    async fn update_permissions_internal(
        &self,
        request: ChangePrivateRoomPermissions,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.update_permissions";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let previous = snapshot
            .room()
            .member(request.target_principal_id)
            .map(agent_room_domain::private_rooms::PrivateRoomMember::permissions)
            .ok_or_else(|| {
                failure(
                    OPERATION,
                    PrivateRoomFailureStage::Validation,
                    PrivateRoomFailureKind::Conflict,
                )
            })?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .update_permissions(
                request.actor.principal_id,
                request.target_principal_id,
                request.permissions,
            )
            .map_err(|error| domain(OPERATION, &error))?;
        let target = self
            .principal_matrix_user(request.target_principal_id, OPERATION)
            .await?;
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;

        match (previous.speak(), request.permissions.speak()) {
            (true, false) => {
                self.set_speaking(&matrix_room, &target, false, OPERATION)
                    .await?;
                self.save_if_changed(&room, expected, changed, OPERATION)
                    .await?;
            }
            (false, true) => {
                self.save_if_changed(&room, expected, changed, OPERATION)
                    .await?;
                self.set_speaking(&matrix_room, &target, true, OPERATION)
                    .await?;
            }
            (_, desired) => {
                self.save_if_changed(&room, expected, changed, OPERATION)
                    .await?;
                if !changed {
                    self.set_speaking(&matrix_room, &target, desired, OPERATION)
                        .await?;
                }
            }
        }
        replace_room(snapshot, room, OPERATION)
    }

    async fn transfer_ownership_internal(
        &self,
        request: TransferPrivateRoomOwnership,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.transfer_ownership";
        let actor_matrix = active_actor_matrix_user(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let target_matrix = self
            .principal_matrix_user(request.target_principal_id, OPERATION)
            .await?;
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;

        if snapshot.room().owner_principal_id() == request.target_principal_id {
            return self
                .reconcile_completed_transfer(
                    snapshot,
                    &request,
                    &matrix_room,
                    &actor_matrix,
                    &target_matrix,
                    OPERATION,
                )
                .await;
        }

        let target_previous_speaks = snapshot
            .room()
            .member(request.target_principal_id)
            .is_some_and(|member| member.permissions().speak());
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .transfer_ownership(
                request.actor.principal_id,
                request.target_principal_id,
                request.former_owner_permissions,
            )
            .map_err(|error| domain(OPERATION, &error))?;
        if !request.former_owner_permissions.speak() {
            self.set_speaking(&matrix_room, &actor_matrix, false, OPERATION)
                .await?;
        }
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        if !target_previous_speaks {
            self.set_speaking(&matrix_room, &target_matrix, true, OPERATION)
                .await?;
        }
        if request.former_owner_permissions.speak() {
            self.set_speaking(&matrix_room, &actor_matrix, true, OPERATION)
                .await?;
        }
        replace_room(snapshot, room, OPERATION)
    }

    async fn archive_internal(
        &self,
        request: ArchivePrivateRoom,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.archive";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .archive(request.actor.principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;
        self.matrix
            .archive(&matrix_room)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        replace_room(snapshot, room, OPERATION)
    }

    async fn prepare_creation(
        &self,
        request: &CreatePrivateRoom,
        owner_matrix_user: MatrixUserId,
        operation: &'static str,
    ) -> PrivateRoomResult<PreparedPrivateRoomCreation> {
        let mut room = PrivateRoom::create(request.catalog_id, request.actor.principal_id);
        let mut invited_principals = BTreeSet::new();
        let mut speaking_assignments = vec![PrivateMatrixSpeakingAssignment::new(
            owner_matrix_user.clone(),
            true,
        )];
        let mut matrix_invites = vec![owner_matrix_user];
        let mut unique_matrix_users = BTreeSet::from([request.actor.matrix_user_id.clone()]);

        for invitation in &request.invitations {
            if invitation.principal_id == request.actor.principal_id
                || !invited_principals.insert(invitation.principal_id)
            {
                return Err(failure(
                    operation,
                    PrivateRoomFailureStage::Validation,
                    PrivateRoomFailureKind::InvalidRequest,
                ));
            }
            room.invite(
                request.actor.principal_id,
                invitation.principal_id,
                invitation.permissions,
            )
            .map_err(|error| domain(operation, &error))?;
            let user_id = self
                .principal_matrix_user(invitation.principal_id, operation)
                .await?;
            if !unique_matrix_users.insert(user_id.as_str().to_owned()) {
                return Err(failure(
                    operation,
                    PrivateRoomFailureStage::Directory,
                    PrivateRoomFailureKind::Conflict,
                ));
            }
            speaking_assignments.push(PrivateMatrixSpeakingAssignment::new(
                user_id.clone(),
                invitation.permissions.speak(),
            ));
            matrix_invites.push(user_id);
        }

        let alias = private_room_alias(request.catalog_id, operation)?;
        let matrix_request = MatrixCreateRoom::new(
            Some(request.name.clone()),
            (!request.description.is_empty()).then(|| request.description.clone()),
            MatrixRoomVisibility::Private,
            MatrixRoomPreset::PrivateChat,
            false,
            matrix_invites,
        )
        .map_err(|error| domain(operation, &error))?
        .with_alias_localpart(alias.clone())
        .with_power_profile(MatrixRoomPowerProfile::ManagedPrivate);
        let matrix = PrivateMatrixRoomCreation::new(matrix_request, alias)
            .map_err(|error| domain(operation, &error))?;
        Ok(PreparedPrivateRoomCreation {
            room,
            matrix,
            speaking_assignments,
        })
    }

    fn build_snapshot(
        &self,
        request: &CreatePrivateRoom,
        room: PrivateRoom,
        matrix_room_id: &MatrixRoomId,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        let catalog = RoomCatalog::new(
            request.catalog_id,
            RoomCatalogFields {
                kind: RoomCatalogKind::PrivateRoom,
                slug: None,
                name: request.name.clone(),
                description: request.description.clone(),
                language: None,
                matrix_space_id: None,
                owner_principal_id: Some(request.actor.principal_id),
                visibility: RoomCatalogVisibility::Private,
                retention_days: request.retention_days,
                status: RoomCatalogStatus::Active,
            },
        )
        .map_err(|error| domain(operation, &error))?;
        let matrix_reference = MatrixRoomReference::new(matrix_room_id.as_str().to_owned())
            .map_err(|error| domain(operation, &error))?;
        let instance = RoomInstance::restore(
            self.identifiers.room_instance_id(),
            RoomInstanceFields {
                catalog_id: request.catalog_id,
                matrix_room_id: matrix_reference,
                region: None,
                capacity: RoomCapacity::standard(),
                projected_member_count: 0,
                allocated_slots: 0,
                activity_score_millis: 0,
                state: RoomInstanceState::Active,
            },
        )
        .map_err(|error| domain(operation, &error))?;
        PrivateRoomSnapshot::new(catalog, instance, room).map_err(|error| domain(operation, &error))
    }

    async fn find(
        &self,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> PrivateRoomResult<Option<PrivateRoomSnapshot>> {
        self.store
            .find_by_catalog(catalog_id)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))
    }

    async fn load(
        &self,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        self.find(catalog_id, operation).await?.ok_or_else(|| {
            failure(
                operation,
                PrivateRoomFailureStage::Persistence,
                PrivateRoomFailureKind::NotFound,
            )
        })
    }

    async fn principal_matrix_user(
        &self,
        principal_id: PrincipalId,
        operation: &'static str,
    ) -> PrivateRoomResult<MatrixUserId> {
        self.principals
            .matrix_user_id(principal_id)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Directory, &error))?
            .ok_or_else(|| {
                failure(
                    operation,
                    PrivateRoomFailureStage::Directory,
                    PrivateRoomFailureKind::NotFound,
                )
            })
    }

    async fn save_if_changed(
        &self,
        room: &PrivateRoom,
        expected: agent_room_domain::version::AggregateVersion,
        changed: bool,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        if !changed {
            return Ok(());
        }
        self.store
            .save(room, expected, self.clock.now())
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))
    }

    async fn require_matrix_membership(
        &self,
        snapshot: &PrivateRoomSnapshot,
        user_id: &MatrixUserId,
        requirement: MatrixMembershipRequirement,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        let matrix_room = matrix_room_id(snapshot, operation)?;
        let membership = self
            .matrix
            .membership(&matrix_room, user_id)
            .await
            .map_err(|error| matrix(operation, error))?;
        if requirement.matches(membership) {
            Ok(())
        } else {
            Err(failure(
                operation,
                PrivateRoomFailureStage::Matrix,
                PrivateRoomFailureKind::Conflict,
            ))
        }
    }

    async fn set_speaking(
        &self,
        room_id: &MatrixRoomId,
        user_id: &MatrixUserId,
        allowed: bool,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        self.matrix
            .set_speaking(room_id, user_id, allowed)
            .await
            .map_err(|error| matrix(operation, error))
    }

    async fn reconcile_completed_transfer(
        &self,
        snapshot: PrivateRoomSnapshot,
        request: &TransferPrivateRoomOwnership,
        matrix_room: &MatrixRoomId,
        actor_matrix: &MatrixUserId,
        target_matrix: &MatrixUserId,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        if request.actor.principal_id == request.target_principal_id {
            self.set_speaking(matrix_room, target_matrix, true, operation)
                .await?;
            return Ok(snapshot);
        }
        let former_matches = snapshot
            .room()
            .member(request.actor.principal_id)
            .is_some_and(|member| member.permissions() == request.former_owner_permissions);
        if !former_matches {
            return Err(failure(
                operation,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::Conflict,
            ));
        }
        self.set_speaking(matrix_room, target_matrix, true, operation)
            .await?;
        self.set_speaking(
            matrix_room,
            actor_matrix,
            request.former_owner_permissions.speak(),
            operation,
        )
        .await?;
        Ok(snapshot)
    }
}

impl PrivateRoomUseCases for PrivateRoomService {
    fn create(
        &self,
        request: CreatePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.create_internal(request))
    }

    fn inspect(
        &self,
        request: InspectPrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.inspect_internal(request))
    }

    fn invite(
        &self,
        request: InvitePrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.invite_internal(request))
    }

    fn accept(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.accept_internal(request))
    }

    fn decline(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.decline_internal(request))
    }

    fn leave(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.leave_internal(request))
    }

    fn remove(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.remove_internal(request))
    }

    fn ban(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.ban_internal(request))
    }

    fn update_permissions(
        &self,
        request: ChangePrivateRoomPermissions,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.update_permissions_internal(request))
    }

    fn transfer_ownership(
        &self,
        request: TransferPrivateRoomOwnership,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.transfer_ownership_internal(request))
    }

    fn archive(
        &self,
        request: ArchivePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.archive_internal(request))
    }
}

#[derive(Debug, Clone, Copy)]
enum MatrixMembershipRequirement {
    Joined,
    AbsentOrLeft,
}

struct PreparedPrivateRoomCreation {
    room: PrivateRoom,
    matrix: PrivateMatrixRoomCreation,
    speaking_assignments: Vec<PrivateMatrixSpeakingAssignment>,
}

impl MatrixMembershipRequirement {
    const fn matches(self, membership: Option<PrivateMatrixMembership>) -> bool {
        match self {
            Self::Joined => matches!(membership, Some(PrivateMatrixMembership::Joined)),
            Self::AbsentOrLeft => {
                matches!(membership, None | Some(PrivateMatrixMembership::Left))
            }
        }
    }
}

fn ensure_same_creation(
    request: &CreatePrivateRoom,
    existing: PrivateRoomSnapshot,
    operation: &'static str,
) -> PrivateRoomResult<PrivateRoomSnapshot> {
    let same = existing.room().owner_principal_id() == request.actor.principal_id
        && existing.catalog().name() == request.name
        && existing.catalog().description() == request.description
        && existing.catalog().retention_days() == request.retention_days;
    if same {
        Ok(existing)
    } else {
        Err(failure(
            operation,
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::Conflict,
        ))
    }
}

fn private_room_alias(
    catalog_id: RoomCatalogId,
    operation: &'static str,
) -> PrivateRoomResult<MatrixRoomAliasLocalpart> {
    MatrixRoomAliasLocalpart::new(format!(
        "agent-room-private-{}",
        catalog_id.as_uuid().simple()
    ))
    .map_err(|_| {
        failure(
            operation,
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::Internal,
        )
    })
}

fn ensure_active_actor(
    actor: &AuthenticatedPrincipal,
    now: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> PrivateRoomResult<()> {
    if now < actor.expires_at {
        Ok(())
    } else {
        Err(failure(
            operation,
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::Forbidden,
        ))
    }
}

fn active_actor_matrix_user(
    actor: &AuthenticatedPrincipal,
    now: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> PrivateRoomResult<MatrixUserId> {
    ensure_active_actor(actor, now, operation)?;
    MatrixUserId::new(actor.matrix_user_id.clone()).map_err(|_| {
        failure(
            operation,
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::Internal,
        )
    })
}

fn matrix_room_id(
    snapshot: &PrivateRoomSnapshot,
    operation: &'static str,
) -> PrivateRoomResult<MatrixRoomId> {
    MatrixRoomId::new(snapshot.instance().matrix_room_id().as_str().to_owned()).map_err(|_| {
        failure(
            operation,
            PrivateRoomFailureStage::Persistence,
            PrivateRoomFailureKind::Internal,
        )
    })
}

fn replace_room(
    snapshot: PrivateRoomSnapshot,
    room: PrivateRoom,
    operation: &'static str,
) -> PrivateRoomResult<PrivateRoomSnapshot> {
    snapshot
        .replacing_room(room)
        .map_err(|error| domain(operation, &error))
}
