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
define_id!(LoginAttemptId);
define_id!(WebSessionId);
define_id!(DeviceId);
define_id!(AgentId);
define_id!(AgentInstanceId);
define_id!(RoomCatalogId);
define_id!(RoomInstanceId);
define_id!(ContentId);
define_id!(HandoffId);
define_id!(AutomationGrantId);
define_id!(OutboxEventId);
