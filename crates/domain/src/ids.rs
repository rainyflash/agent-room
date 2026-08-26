use std::fmt::{Display, Formatter};

use uuid::Uuid;

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);

        impl $name {
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self::from_uuid(value)
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

define_id!(PrincipalId);
define_id!(AccountDeletionJobId);
define_id!(LoginAttemptId);
define_id!(WebSessionId);
define_id!(DeviceId);
define_id!(DeviceTokenFamilyId);
define_id!(DeviceAccessTokenId);
define_id!(DeviceRefreshTokenId);
define_id!(AgentId);
define_id!(AgentCreationRequestId);
define_id!(AgentCardSnapshotId);
define_id!(AdapterBindingId);
define_id!(AgentInstanceId);
define_id!(AgentInstanceRegistrationRequestId);
define_id!(RoomCatalogId);
define_id!(RoomInstanceId);
define_id!(RoomReservationId);
define_id!(RoomProvisioningJobId);
define_id!(RoomProvisioningLeaseId);
define_id!(ContentId);
define_id!(ContentUploadRequestId);
define_id!(ContentEncryptionContextId);
define_id!(MessageId);
define_id!(MessageRevisionId);
define_id!(MessageSubmissionId);
define_id!(HandoffId);
define_id!(AutomationGrantId);
define_id!(ModerationCaseId);
define_id!(ModerationActionId);
define_id!(AuditEventId);
define_id!(FederationRuleId);
define_id!(OutboxEventId);
