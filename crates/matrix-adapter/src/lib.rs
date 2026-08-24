//! Matrix Client-Server API 基础设施适配器。

mod configuration;
mod error;
mod mapping;
mod membership;
mod sdk;

pub use configuration::{
    MatrixSdkConfiguration, MatrixSdkConfigurationError, MatrixSdkStoreConfiguration,
    MatrixSdkStoreConfigurationError,
};
pub use membership::MatrixRoomMembershipAdapter;
pub use sdk::MatrixSdkClientFactory;
