use agent_room_application::{
    ports::PrivateRoomSnapshot,
    private_rooms::{CreatePrivateRoom, PrivateRoomInvitation},
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::{PrivateRoomCapability, PrivateRoomMember, PrivateRoomPermissions},
};
use serde::{Deserialize, Serialize};

use crate::features::resource_ids::parse_uuid_v7;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CreatePrivateRoomBody {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    retention_days: Option<u16>,
    #[serde(default)]
    invitations: Vec<InvitationBody>,
}

impl CreatePrivateRoomBody {
    pub(super) fn into_request(
        self,
        actor: agent_room_application::authentication::AuthenticatedPrincipal,
        catalog_id: RoomCatalogId,
    ) -> Option<CreatePrivateRoom> {
        let invitations = self
            .invitations
            .into_iter()
            .map(InvitationBody::into_invitation)
            .collect::<Option<Vec<_>>>()?;
        Some(CreatePrivateRoom {
            actor,
            catalog_id,
            name: self.name,
            description: self.description,
            retention_days: self.retention_days,
            invitations,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InvitationBody {
    principal_id: String,
    permissions: PermissionsBody,
}

impl InvitationBody {
    fn into_invitation(self) -> Option<PrivateRoomInvitation> {
        Some(PrivateRoomInvitation {
            principal_id: principal_id(&self.principal_id)?,
            permissions: self.permissions.into_domain()?,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct InviteMemberBody {
    pub(super) target_principal_id: String,
    pub(super) permissions: PermissionsBody,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PermissionsBody {
    capabilities: Vec<PermissionCapability>,
}

impl PermissionsBody {
    pub(super) fn into_domain(self) -> Option<PrivateRoomPermissions> {
        PrivateRoomPermissions::from_capabilities(
            self.capabilities
                .into_iter()
                .map(PermissionCapability::into_domain),
        )
        .ok()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct TransferOwnershipBody {
    pub(super) target_principal_id: String,
    pub(super) former_owner_permissions: PermissionsBody,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PrivateRoomResponse {
    catalog_id: String,
    room_instance_id: String,
    matrix_room_id: String,
    name: String,
    description: String,
    retention_days: Option<u16>,
    owner_principal_id: String,
    status: &'static str,
    version: i64,
    members: Vec<PrivateRoomMemberResponse>,
}

impl From<PrivateRoomSnapshot> for PrivateRoomResponse {
    fn from(snapshot: PrivateRoomSnapshot) -> Self {
        let members = snapshot
            .room()
            .members()
            .map(PrivateRoomMemberResponse::from)
            .collect();
        Self {
            catalog_id: snapshot.catalog().id().to_string(),
            room_instance_id: snapshot.instance().id().to_string(),
            matrix_room_id: snapshot.instance().matrix_room_id().as_str().to_owned(),
            name: snapshot.catalog().name().to_owned(),
            description: snapshot.catalog().description().to_owned(),
            retention_days: snapshot.catalog().retention_days(),
            owner_principal_id: snapshot.room().owner_principal_id().to_string(),
            status: snapshot.room().status().as_str(),
            version: snapshot.room().version().value(),
            members,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrivateRoomMemberResponse {
    principal_id: String,
    status: &'static str,
    permissions: PermissionsResponse,
}

impl From<&PrivateRoomMember> for PrivateRoomMemberResponse {
    fn from(member: &PrivateRoomMember) -> Self {
        Self {
            principal_id: member.principal_id().to_string(),
            status: member.status().as_str(),
            permissions: PermissionsResponse::from(member.permissions()),
        }
    }
}

#[derive(Debug, Serialize)]
struct PermissionsResponse {
    capabilities: Vec<PermissionCapability>,
}

impl From<PrivateRoomPermissions> for PermissionsResponse {
    fn from(permissions: PrivateRoomPermissions) -> Self {
        let candidates = [
            (PrivateRoomCapability::View, PermissionCapability::View),
            (PrivateRoomCapability::Speak, PermissionCapability::Speak),
            (PrivateRoomCapability::Invite, PermissionCapability::Invite),
            (PrivateRoomCapability::Manage, PermissionCapability::Manage),
            (
                PrivateRoomCapability::Automate,
                PermissionCapability::Automate,
            ),
        ];
        Self {
            capabilities: candidates
                .into_iter()
                .filter_map(|(domain, wire)| permissions.allows(domain).then_some(wire))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PermissionCapability {
    View,
    Speak,
    Invite,
    Manage,
    Automate,
}

impl PermissionCapability {
    const fn into_domain(self) -> PrivateRoomCapability {
        match self {
            Self::View => PrivateRoomCapability::View,
            Self::Speak => PrivateRoomCapability::Speak,
            Self::Invite => PrivateRoomCapability::Invite,
            Self::Manage => PrivateRoomCapability::Manage,
            Self::Automate => PrivateRoomCapability::Automate,
        }
    }
}

pub(super) fn catalog_id(value: &str) -> Option<RoomCatalogId> {
    parse_uuid_v7(value).map(RoomCatalogId::from_uuid).ok()
}

pub(super) fn principal_id(value: &str) -> Option<PrincipalId> {
    parse_uuid_v7(value).map(PrincipalId::from_uuid).ok()
}
