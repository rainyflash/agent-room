use std::collections::HashMap;

use agent_room_application::ports::{
    ContentByteStream, ContentStreamFailure, ContentStreamFailureKind, ObjectStoreFailure,
    ObjectStoreFailureKind, ObjectStoreResult, ObjectWriteReceipt, OpenedContentObject,
    PrivateContentObjectStore,
};
use agent_room_domain::content::{ContentByteLength, ContentObject, Sha256Digest};
use aws_sdk_s3::{
    Client,
    config::{
        BehaviorVersion, Credentials, Region, RequestChecksumCalculation,
        ResponseChecksumValidation, retry::RetryConfig, timeout::TimeoutConfig,
    },
    primitives::ByteStream,
};
use futures_util::stream;
use thiserror::Error;

use crate::{
    S3ContentStoreConfig,
    encoding::{decode_sha256, lower_hex},
    spool::SpooledContent,
};

const DIGEST_METADATA_KEY: &str = "sha256";
const LENGTH_METADATA_KEY: &str = "byte-length";
const CONTENT_ID_METADATA_KEY: &str = "content-id";

#[derive(Debug, Clone)]
pub struct S3PrivateContentObjectStore {
    client: Client,
    bucket: String,
}

impl S3PrivateContentObjectStore {
    pub fn new(configuration: &S3ContentStoreConfig) -> Self {
        let timeout = configuration.operation_timeout();
        let timeout_config = TimeoutConfig::builder()
            .connect_timeout(timeout.min(std::time::Duration::from_secs(10)))
            .operation_attempt_timeout(timeout)
            .operation_timeout(timeout)
            .build();
        let credentials = Credentials::new(
            configuration.access_key_id().expose(),
            configuration.secret_access_key().expose(),
            None,
            None,
            "agent-room-static",
        );
        let sdk_configuration = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .endpoint_url(configuration.endpoint().as_str())
            .region(Region::new(configuration.region().to_owned()))
            .credentials_provider(credentials)
            .force_path_style(true)
            .retry_config(RetryConfig::standard().with_max_attempts(2))
            .timeout_config(timeout_config)
            // SeaweedFS 与部分 S3 兼容实现不接受 SDK 的可选校验和尾部帧。
            // 应用层仍会独立计算并验证 SHA-256，不依赖此兼容开关保证完整性。
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
            .build();
        Self {
            client: Client::from_conf(sdk_configuration),
            bucket: configuration.bucket().to_owned(),
        }
    }

    /// 核验内容桶，并在明确授权时创建缺失桶。
    ///
    /// # Errors
    ///
    /// 桶缺失但未授权创建，或 S3 兼容端点不可用时返回错误。
    pub async fn ensure_bucket(
        &self,
        create_if_missing: bool,
    ) -> Result<(), S3BucketProvisionError> {
        match self.client.head_bucket().bucket(&self.bucket).send().await {
            Ok(_) => return Ok(()),
            Err(error) if response_status(&error) == Some(404) && create_if_missing => {}
            Err(error) if response_status(&error) == Some(404) => {
                return Err(S3BucketProvisionError::Missing);
            }
            Err(_) => return Err(S3BucketProvisionError::Unavailable),
        }

        match self
            .client
            .create_bucket()
            .bucket(&self.bucket)
            .send()
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if response_status(&error) == Some(409) => self
                .client
                .head_bucket()
                .bucket(&self.bucket)
                .send()
                .await
                .map(|_| ())
                .map_err(|_| S3BucketProvisionError::Unavailable),
            Err(_) => Err(S3BucketProvisionError::CreateRejected),
        }
    }

    async fn put_internal(
        &self,
        content: &ContentObject,
        body: ContentByteStream,
    ) -> ObjectStoreResult<ObjectWriteReceipt> {
        let spooled = SpooledContent::collect(body).await?;
        verify_declaration(content, &spooled)?;
        let request_body = ByteStream::from_path(spooled.path())
            .await
            .map_err(|_| unavailable("写入私有对象"))?;
        let byte_length =
            i64::try_from(spooled.byte_length().value()).map_err(|_| rejected("写入私有对象"))?;
        let digest = lower_hex(spooled.digest().as_bytes());

        let result = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(content.storage_key().as_str())
            .body(request_body)
            .content_length(byte_length)
            .content_type(content.media_type().as_str())
            .metadata(DIGEST_METADATA_KEY, digest)
            .metadata(LENGTH_METADATA_KEY, byte_length.to_string())
            .metadata(CONTENT_ID_METADATA_KEY, content.id().to_string())
            .if_none_match("*")
            .send()
            .await;

        match result {
            Ok(_) => Ok(receipt(&spooled)),
            Err(error) if response_status(&error) == Some(412) => {
                self.verify_existing(content).await?;
                Ok(receipt(&spooled))
            }
            Err(_) => Err(unavailable("写入私有对象")),
        }
    }

    async fn verify_existing(&self, content: &ContentObject) -> ObjectStoreResult<()> {
        let output = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(content.storage_key().as_str())
            .send()
            .await
            .map_err(|_| unavailable("核验既有私有对象"))?;
        let digest = parse_digest(output.metadata())?;
        let byte_length = parse_length(output.metadata(), output.content_length())?;
        if digest != Some(content.digest()) || byte_length != Some(content.byte_length()) {
            return Err(corrupt_metadata("核验既有私有对象"));
        }
        Ok(())
    }

    async fn open_internal(
        &self,
        content: &ContentObject,
    ) -> ObjectStoreResult<OpenedContentObject> {
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(content.storage_key().as_str())
            .send()
            .await
            .map_err(|error| {
                if response_status(&error) == Some(404) {
                    ObjectStoreFailure::new("读取私有对象", ObjectStoreFailureKind::NotFound)
                } else {
                    unavailable("读取私有对象")
                }
            })?;

        let reported_digest = parse_digest(output.metadata())?;
        let reported_byte_length = parse_length(output.metadata(), output.content_length())?;
        let body = output.body;
        let stream = stream::unfold(Some(body), |state| async move {
            let mut body = state?;
            match body.try_next().await {
                Ok(Some(chunk)) => Some((Ok(chunk.to_vec()), Some(body))),
                Ok(None) => None,
                Err(_) => Some((
                    Err(ContentStreamFailure::new(
                        "读取私有对象",
                        ContentStreamFailureKind::StorageUnavailable,
                    )),
                    None,
                )),
            }
        });

        Ok(OpenedContentObject {
            reported_digest,
            reported_byte_length,
            body: Box::pin(stream),
        })
    }

    async fn delete_internal(&self, content: &ContentObject) -> ObjectStoreResult<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(content.storage_key().as_str())
            .send()
            .await
            .map_err(|_| unavailable("删除私有对象"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum S3BucketProvisionError {
    #[error("对象存储桶不存在，且当前部署未授权自动创建")]
    Missing,
    #[error("对象存储端点不可用或凭据无权核验桶")]
    Unavailable,
    #[error("对象存储拒绝创建内容桶")]
    CreateRejected,
}

impl PrivateContentObjectStore for S3PrivateContentObjectStore {
    fn put<'a>(
        &'a self,
        content: &'a ContentObject,
        body: ContentByteStream,
    ) -> agent_room_application::ports::PortFuture<'a, ObjectStoreResult<ObjectWriteReceipt>> {
        Box::pin(self.put_internal(content, body))
    }

    fn open<'a>(
        &'a self,
        content: &'a ContentObject,
    ) -> agent_room_application::ports::PortFuture<'a, ObjectStoreResult<OpenedContentObject>> {
        Box::pin(self.open_internal(content))
    }

    fn delete<'a>(
        &'a self,
        content: &'a ContentObject,
    ) -> agent_room_application::ports::PortFuture<'a, ObjectStoreResult<()>> {
        Box::pin(self.delete_internal(content))
    }
}

fn verify_declaration(content: &ContentObject, spooled: &SpooledContent) -> ObjectStoreResult<()> {
    if spooled.digest() != content.digest() || spooled.byte_length() != content.byte_length() {
        return Err(rejected("核验上传声明"));
    }
    Ok(())
}

const fn receipt(spooled: &SpooledContent) -> ObjectWriteReceipt {
    ObjectWriteReceipt {
        digest: spooled.digest(),
        byte_length: spooled.byte_length(),
    }
}

fn parse_digest(
    metadata: Option<&HashMap<String, String>>,
) -> ObjectStoreResult<Option<Sha256Digest>> {
    metadata
        .and_then(|metadata| metadata.get(DIGEST_METADATA_KEY))
        .map(|value| {
            decode_sha256(value)
                .map(Sha256Digest::from_bytes)
                .ok_or_else(|| corrupt_metadata("解析私有对象元数据"))
        })
        .transpose()
}

fn parse_length(
    metadata: Option<&HashMap<String, String>>,
    content_length: Option<i64>,
) -> ObjectStoreResult<Option<ContentByteLength>> {
    let header_length = content_length
        .map(|value| {
            u64::try_from(value)
                .ok()
                .and_then(|value| ContentByteLength::new(value).ok())
                .ok_or_else(|| corrupt_metadata("解析私有对象元数据"))
        })
        .transpose()?;
    let metadata_length = metadata
        .and_then(|metadata| metadata.get(LENGTH_METADATA_KEY))
        .map(|value| {
            value
                .parse::<u64>()
                .ok()
                .and_then(|value| ContentByteLength::new(value).ok())
                .ok_or_else(|| corrupt_metadata("解析私有对象元数据"))
        })
        .transpose()?;

    if header_length.is_some() && metadata_length.is_some() && header_length != metadata_length {
        return Err(corrupt_metadata("解析私有对象元数据"));
    }
    Ok(metadata_length.or(header_length))
}

fn response_status<E>(error: &aws_sdk_s3::error::SdkError<E>) -> Option<u16> {
    error
        .raw_response()
        .map(|response| response.status().as_u16())
}

const fn unavailable(operation: &'static str) -> ObjectStoreFailure {
    ObjectStoreFailure::new(operation, ObjectStoreFailureKind::Unavailable)
}

const fn rejected(operation: &'static str) -> ObjectStoreFailure {
    ObjectStoreFailure::new(operation, ObjectStoreFailureKind::Rejected)
}

const fn corrupt_metadata(operation: &'static str) -> ObjectStoreFailure {
    ObjectStoreFailure::new(operation, ObjectStoreFailureKind::CorruptMetadata)
}
