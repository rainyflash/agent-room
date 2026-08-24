//! Matrix Client-Server API 基础设施适配器。

mod configuration;
mod error;
mod mapping;
mod provisioning;
mod sdk;

pub use configuration::{MatrixSdkConfiguration, MatrixSdkConfigurationError};
pub use provisioning::{
    MatrixApplicationServiceConfiguration, MatrixApplicationServiceConfigurationError,
    MatrixApplicationServiceProvisioner,
};
pub use sdk::MatrixSdkClientFactory;
