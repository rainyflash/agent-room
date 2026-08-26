use std::{io, path::PathBuf, time::SystemTimeError};

use agent_room_release_manifest::ReleaseManifestError;
use thiserror::Error;

pub type ToolResult<T> = Result<T, ToolError>;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("路径已经存在，拒绝覆盖：{0}")]
    RefuseOverwrite(PathBuf),
    #[error("无法访问 {path}：{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("JSON 无效：{0}")]
    Json(#[from] serde_json::Error),
    #[error("发布策略无效：{0}")]
    Manifest(#[from] ReleaseManifestError),
    #[error("密钥文档架构或算法不受支持")]
    UnsupportedKeyDocument,
    #[error("密钥编码无效")]
    InvalidKeyEncoding,
    #[error("密钥长度无效")]
    InvalidKeyLength,
    #[error("公钥与私钥标识不一致")]
    KeyIdMismatch,
    #[error("操作系统随机数生成失败")]
    RandomSource,
    #[error("系统时间无效：{0}")]
    SystemTime(#[from] SystemTimeError),
}
