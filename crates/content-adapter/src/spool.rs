use std::path::Path;

use agent_room_application::ports::{
    ContentByteStream, ContentStreamFailure, ContentStreamFailureKind, ObjectStoreFailure,
    ObjectStoreFailureKind, ObjectStoreResult,
};
use agent_room_domain::content::{ContentByteLength, MAX_CONTENT_BYTES, Sha256Digest};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;

#[derive(Debug)]
pub(crate) struct SpooledContent {
    file: NamedTempFile,
    digest: Sha256Digest,
    byte_length: ContentByteLength,
}

impl SpooledContent {
    pub(crate) async fn collect(mut body: ContentByteStream) -> ObjectStoreResult<Self> {
        let file = NamedTempFile::new().map_err(|_| unavailable("暂存上传"))?;
        let writer = file.reopen().map_err(|_| unavailable("暂存上传"))?;
        let mut writer = tokio::fs::File::from_std(writer);
        let mut hasher = Sha256::new();
        let mut byte_length = 0_u64;

        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|failure| map_source_failure(&failure))?;
            let chunk_length = u64::try_from(chunk.len()).map_err(|_| rejected("暂存上传"))?;
            byte_length = byte_length
                .checked_add(chunk_length)
                .ok_or_else(|| rejected("暂存上传"))?;
            if byte_length > MAX_CONTENT_BYTES {
                return Err(rejected("暂存上传"));
            }
            writer
                .write_all(&chunk)
                .await
                .map_err(|_| unavailable("暂存上传"))?;
            hasher.update(&chunk);
        }
        writer.flush().await.map_err(|_| unavailable("暂存上传"))?;
        drop(writer);

        let byte_length = ContentByteLength::new(byte_length).map_err(|_| rejected("暂存上传"))?;
        let digest = Sha256Digest::from_bytes(hasher.finalize().into());
        Ok(Self {
            file,
            digest,
            byte_length,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        self.file.path()
    }

    pub(crate) const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    pub(crate) const fn byte_length(&self) -> ContentByteLength {
        self.byte_length
    }
}

fn map_source_failure(failure: &ContentStreamFailure) -> ObjectStoreFailure {
    let kind = match failure.kind() {
        ContentStreamFailureKind::SizeLimitExceeded
        | ContentStreamFailureKind::IntegrityMismatch => ObjectStoreFailureKind::Rejected,
        ContentStreamFailureKind::Source | ContentStreamFailureKind::StorageUnavailable => {
            ObjectStoreFailureKind::Unavailable
        }
    };
    ObjectStoreFailure::new("暂存上传", kind)
}

const fn unavailable(operation: &'static str) -> ObjectStoreFailure {
    ObjectStoreFailure::new(operation, ObjectStoreFailureKind::Unavailable)
}

const fn rejected(operation: &'static str) -> ObjectStoreFailure {
    ObjectStoreFailure::new(operation, ObjectStoreFailureKind::Rejected)
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{ContentByteStream, ObjectStoreFailureKind};
    use futures_util::stream;
    use sha2::{Digest, Sha256};

    use super::SpooledContent;

    #[tokio::test]
    async fn 分块上传被流式写入并计算权威摘要() {
        let body: ContentByteStream =
            Box::pin(stream::iter([Ok(b"agent ".to_vec()), Ok(b"room".to_vec())]));
        let spooled = SpooledContent::collect(body).await.expect("暂存成功");
        let expected: [u8; 32] = Sha256::digest(b"agent room").into();

        assert_eq!(spooled.byte_length().value(), 10);
        assert_eq!(spooled.digest().as_bytes(), &expected);
        assert!(spooled.path().exists());
    }

    #[tokio::test]
    async fn 空流被拒绝而不是创建无效对象() {
        let body: ContentByteStream = Box::pin(stream::empty());
        let failure = SpooledContent::collect(body)
            .await
            .expect_err("空内容必须失败");
        assert_eq!(failure.kind(), ObjectStoreFailureKind::Rejected);
    }
}
