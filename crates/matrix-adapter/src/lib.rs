//! Matrix Client-Server API 基础设施适配器。

mod configuration;
mod error;
mod mapping;
mod sdk;

pub use configuration::{
    MatrixSdkConfiguration, MatrixSdkConfigurationError, MatrixSdkStoreConfiguration,
    MatrixSdkStoreConfigurationError,
};
pub use sdk::MatrixSdkClientFactory;
