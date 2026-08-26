use thiserror::Error;

pub type ReleaseManifestResult<T> = Result<T, ReleaseManifestError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReleaseManifestError {
    #[error("发布清单算法不受支持")]
    UnsupportedAlgorithm,
    #[error("发布清单密钥标识不匹配")]
    UntrustedKey,
    #[error("发布清单载荷编码无效")]
    InvalidPayloadEncoding,
    #[error("发布清单签名编码无效")]
    InvalidSignatureEncoding,
    #[error("发布清单签名无效")]
    InvalidSignature,
    #[error("发布清单 JSON 无效")]
    InvalidPayload,
    #[error("发布清单架构版本不受支持")]
    UnsupportedSchema,
    #[error("发布渠道不匹配")]
    ChannelMismatch,
    #[error("发布时间超出允许的时钟偏差")]
    PublishedInFuture,
    #[error("发布清单已经过期")]
    Expired,
    #[error("发布清单有效期无效")]
    InvalidLifetime,
    #[error("发布序号没有单调递增")]
    StaleSequence,
    #[error("发布版本无效")]
    InvalidVersion,
    #[error("发布版本不是更新版本")]
    VersionNotNewer,
    #[error("发布降级未被离线签名明确授权")]
    UnauthorizedRollback,
    #[error("发布清单没有产物")]
    MissingArtifacts,
    #[error("发布产物名称、类型和平台的组合必须唯一")]
    DuplicateArtifact,
    #[error("发布产物名称无效")]
    InvalidArtifactName,
    #[error("发布产物地址无效")]
    InvalidArtifactUrl,
    #[error("发布产物摘要无效")]
    InvalidArtifactDigest,
    #[error("发布产物大小无效")]
    InvalidArtifactSize,
    #[error("发布产物证明地址无效")]
    InvalidAttestationUrl,
    #[error("桌面更新缺少 Tauri 更新清单")]
    MissingTauriManifest,
    #[error("Tauri 更新清单地址无效")]
    InvalidTauriManifestUrl,
}
