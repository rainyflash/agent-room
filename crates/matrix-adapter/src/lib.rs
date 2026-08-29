//! Matrix Client-Server API 基础设施适配器。

mod configuration;
mod error;
mod handoff;
mod mapping;
mod membership;
mod provisioning;
mod sdk;
mod store_recovery;

pub use configuration::{
    MatrixSdkConfiguration, MatrixSdkConfigurationError, MatrixSdkStoreConfiguration,
    MatrixSdkStoreConfigurationError,
};
pub use handoff::MatrixSdkHandoffGateway;
pub use membership::MatrixRoomMembershipAdapter;
pub use provisioning::MatrixRoomProvisioningAdapter;
pub use sdk::{MatrixSdkClientFactory, MatrixSdkHandoffConnection};
