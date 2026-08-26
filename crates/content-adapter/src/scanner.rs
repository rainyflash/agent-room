use std::sync::Arc;

use agent_room_application::ports::{
    ContentScanFailure, ContentScanFailureKind, ContentScanResult, ContentScanner,
    ContentStreamFailureKind, PortFuture, PrivateContentObjectStore,
};
use agent_room_domain::content::{ContentObject, ContentScanState, Sha256Digest};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, lookup_host},
    time::timeout,
};

use crate::{ClamAvScannerConfig, config::is_private_address};

const INSTREAM_COMMAND: &[u8] = b"zINSTREAM\0";
const MAX_PROTOCOL_CHUNK_BYTES: usize = 64 * 1_024;
const MAX_RESPONSE_BYTES: usize = 4 * 1_024;

pub struct ClamAvContentScanner {
    configuration: ClamAvScannerConfig,
    object_store: Arc<dyn PrivateContentObjectStore>,
}

impl ClamAvContentScanner {
    pub const fn new(
        configuration: ClamAvScannerConfig,
        object_store: Arc<dyn PrivateContentObjectStore>,
    ) -> Self {
        Self {
            configuration,
            object_store,
        }
    }

    async fn scan_internal(&self, content: &ContentObject) -> ContentScanResult<ContentScanState> {
        let opened = self
            .object_store
            .open(content)
            .await
            .map_err(|_| unavailable("content.scan.open_object"))?;
        if opened.reported_digest != Some(content.digest())
            || opened.reported_byte_length != Some(content.byte_length())
        {
            return Err(invalid_response("content.scan.object_metadata"));
        }

        let connection = timeout(self.configuration.connect_timeout(), async {
            let addresses = lookup_host(self.configuration.address())
                .await
                .map_err(|_| unavailable("content.scan.resolve"))?
                .collect::<Vec<_>>();
            if addresses.is_empty()
                || addresses
                    .iter()
                    .any(|address| !is_private_address(address.ip()))
            {
                return Err(unavailable("content.scan.resolve"));
            }
            TcpStream::connect(addresses[0])
                .await
                .map_err(|_| unavailable("content.scan.connect"))
        })
        .await
        .map_err(|_| unavailable("content.scan.connect"))??;
        timeout(
            self.configuration.scan_timeout(),
            scan_stream(connection, opened.body, content),
        )
        .await
        .map_err(|_| unavailable("content.scan.timeout"))?
    }
}

impl ContentScanner for ClamAvContentScanner {
    fn scan<'a>(
        &'a self,
        content: &'a ContentObject,
    ) -> PortFuture<'a, ContentScanResult<ContentScanState>> {
        Box::pin(self.scan_internal(content))
    }
}

async fn scan_stream(
    mut connection: TcpStream,
    mut body: agent_room_application::ports::ContentByteStream,
    content: &ContentObject,
) -> ContentScanResult<ContentScanState> {
    connection
        .write_all(INSTREAM_COMMAND)
        .await
        .map_err(|_| unavailable("content.scan.write_command"))?;
    let mut hasher = Sha256::new();
    let mut observed_length = 0_u64;
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|failure| match failure.kind() {
            ContentStreamFailureKind::StorageUnavailable | ContentStreamFailureKind::Source => {
                unavailable("content.scan.read_object")
            }
            ContentStreamFailureKind::SizeLimitExceeded
            | ContentStreamFailureKind::IntegrityMismatch => {
                invalid_response("content.scan.read_object")
            }
        })?;
        observed_length = observed_length
            .checked_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX))
            .ok_or_else(|| invalid_response("content.scan.object_integrity"))?;
        if observed_length > content.byte_length().value() {
            return Err(invalid_response("content.scan.object_integrity"));
        }
        hasher.update(&chunk);
        for protocol_chunk in chunk.chunks(MAX_PROTOCOL_CHUNK_BYTES) {
            let length = u32::try_from(protocol_chunk.len())
                .map_err(|_| invalid_response("content.scan.frame"))?;
            connection
                .write_all(&length.to_be_bytes())
                .await
                .map_err(|_| unavailable("content.scan.write_frame"))?;
            connection
                .write_all(protocol_chunk)
                .await
                .map_err(|_| unavailable("content.scan.write_frame"))?;
        }
    }
    let observed_digest = Sha256Digest::from_bytes(hasher.finalize().into());
    if observed_length != content.byte_length().value() || observed_digest != content.digest() {
        return Err(invalid_response("content.scan.object_integrity"));
    }
    connection
        .write_all(&0_u32.to_be_bytes())
        .await
        .map_err(|_| unavailable("content.scan.finish_stream"))?;
    connection
        .shutdown()
        .await
        .map_err(|_| unavailable("content.scan.finish_stream"))?;
    parse_response(read_response(&mut connection).await?)
}

async fn read_response(connection: &mut TcpStream) -> ContentScanResult<Vec<u8>> {
    let mut response = Vec::new();
    loop {
        let mut chunk = [0_u8; 512];
        let read = connection
            .read(&mut chunk)
            .await
            .map_err(|_| unavailable("content.scan.read_response"))?;
        if read == 0 {
            return Err(invalid_response("content.scan.read_response"));
        }
        let remaining = MAX_RESPONSE_BYTES.saturating_sub(response.len());
        if read > remaining {
            return Err(invalid_response("content.scan.read_response"));
        }
        response.extend_from_slice(&chunk[..read]);
        if response.last() == Some(&0) {
            return Ok(response);
        }
    }
}

fn parse_response(mut response: Vec<u8>) -> ContentScanResult<ContentScanState> {
    if response.pop() != Some(0) || response.contains(&0) {
        return Err(invalid_response("content.scan.parse_response"));
    }
    let response = std::str::from_utf8(&response)
        .map_err(|_| invalid_response("content.scan.parse_response"))?;
    if response.ends_with(" OK") {
        Ok(ContentScanState::Clean)
    } else if response.ends_with(" FOUND") {
        Ok(ContentScanState::Rejected)
    } else {
        Err(invalid_response("content.scan.parse_response"))
    }
}

const fn unavailable(operation: &'static str) -> ContentScanFailure {
    ContentScanFailure::new(operation, ContentScanFailureKind::Unavailable)
}

const fn invalid_response(operation: &'static str) -> ContentScanFailure {
    ContentScanFailure::new(operation, ContentScanFailureKind::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use agent_room_application::ports::{
        ContentByteStream, ContentScanFailureKind, ContentScanner, ObjectStoreResult,
        ObjectWriteReceipt, OpenedContentObject, PortFuture, PrivateContentObjectStore,
    };
    use agent_room_domain::{
        content::{
            ContentByteLength, ContentEncryptionMode, ContentLifecycleState, ContentMediaType,
            ContentObject, ContentObjectFields, ContentScanState, ContentStorageKey, Sha256Digest,
        },
        ids::{ContentId, PrincipalId},
        time::UtcMillis,
    };
    use futures_util::stream;
    use sha2::{Digest, Sha256};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    use uuid::Uuid;

    use super::{ClamAvContentScanner, ClamAvScannerConfig, INSTREAM_COMMAND};

    #[tokio::test]
    async fn 使用_instream_发送完整对象并解析干净结果() {
        let payload = b"clean streamed object".to_vec();
        let (configuration, received) = fake_clamd(b"stream: OK\0").await;
        let scanner = scanner(configuration, payload.clone(), None);

        assert_eq!(
            scanner.scan(&content(&payload)).await.expect("扫描成功"),
            ContentScanState::Clean
        );
        assert_eq!(received.await.expect("服务任务完成"), payload);
    }

    #[tokio::test]
    async fn 病毒命中被拒绝且畸形响应不会放行() {
        let payload = b"EICAR test fixture".to_vec();
        let (found_configuration, found_received) =
            fake_clamd(b"stream: Win.Test.EICAR_HDB-1 FOUND\0").await;
        let found = scanner(found_configuration, payload.clone(), None)
            .scan(&content(&payload))
            .await
            .expect("病毒响应有效");
        assert_eq!(found, ContentScanState::Rejected);
        found_received.await.expect("服务任务完成");

        let (invalid_configuration, invalid_received) = fake_clamd(b"stream: MAYBE\0").await;
        let failure = scanner(invalid_configuration, payload.clone(), None)
            .scan(&content(&payload))
            .await
            .expect_err("未知响应不能放行");
        assert_eq!(failure.kind(), ContentScanFailureKind::InvalidResponse);
        invalid_received.await.expect("服务任务完成");
    }

    #[tokio::test]
    async fn 对象正文与已声明摘要不一致时不会提交扫描终止帧() {
        let declared = b"declared object".to_vec();
        let actual = b"tampered object".to_vec();
        let (configuration, received) = fake_clamd(b"stream: OK\0").await;
        let failure = scanner(configuration, actual, Some(declared.clone()))
            .scan(&content(&declared))
            .await
            .expect_err("篡改对象不得标记为干净");
        assert_eq!(failure.kind(), ContentScanFailureKind::InvalidResponse);
        assert_eq!(received.await.expect("服务任务完成"), b"tampered object");
    }

    fn scanner(
        configuration: ClamAvScannerConfig,
        actual: Vec<u8>,
        declared: Option<Vec<u8>>,
    ) -> ClamAvContentScanner {
        let metadata = declared.unwrap_or_else(|| actual.clone());
        ClamAvContentScanner::new(
            configuration,
            Arc::new(MemoryObjectStore { actual, metadata }),
        )
    }

    async fn fake_clamd(
        response: &'static [u8],
    ) -> (ClamAvScannerConfig, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试端口");
        let address = listener.local_addr().expect("测试地址有效");
        let task = tokio::spawn(async move {
            let (mut connection, _) = listener.accept().await.expect("接受扫描连接");
            let mut command = vec![0_u8; INSTREAM_COMMAND.len()];
            connection.read_exact(&mut command).await.expect("读取命令");
            assert_eq!(command, INSTREAM_COMMAND);
            let mut body = Vec::new();
            loop {
                let Ok(length) = connection.read_u32().await else {
                    return body;
                };
                if length == 0 {
                    connection.write_all(response).await.expect("写入响应");
                    return body;
                }
                let offset = body.len();
                let length = usize::try_from(length).expect("帧长度可转换");
                body.resize(offset + length, 0);
                connection
                    .read_exact(&mut body[offset..])
                    .await
                    .expect("读取帧正文");
            }
        });
        (
            ClamAvScannerConfig::new(
                address.to_string(),
                Duration::from_secs(1),
                Duration::from_secs(2),
            )
            .expect("测试配置有效"),
            task,
        )
    }

    struct MemoryObjectStore {
        actual: Vec<u8>,
        metadata: Vec<u8>,
    }

    impl PrivateContentObjectStore for MemoryObjectStore {
        fn put<'a>(
            &'a self,
            _content: &'a ContentObject,
            _body: ContentByteStream,
        ) -> PortFuture<'a, ObjectStoreResult<ObjectWriteReceipt>> {
            unreachable!("扫描测试不写入对象")
        }

        fn open<'a>(
            &'a self,
            _content: &'a ContentObject,
        ) -> PortFuture<'a, ObjectStoreResult<OpenedContentObject>> {
            Box::pin(async move {
                Ok(OpenedContentObject {
                    reported_digest: Some(digest(&self.metadata)),
                    reported_byte_length: Some(length(&self.metadata)),
                    body: Box::pin(stream::iter([Ok(self.actual.clone())])),
                })
            })
        }

        fn delete<'a>(
            &'a self,
            _content: &'a ContentObject,
        ) -> PortFuture<'a, ObjectStoreResult<()>> {
            unreachable!("扫描测试不删除对象")
        }
    }

    fn content(payload: &[u8]) -> ContentObject {
        ContentObject::restore(ContentObjectFields {
            id: ContentId::from_uuid(Uuid::now_v7()),
            owner_principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
            storage_key: ContentStorageKey::new(format!(
                "content/v1/test/{}/opaque-object-key",
                Uuid::now_v7()
            ))
            .expect("对象键有效"),
            digest: digest(payload),
            byte_length: length(payload),
            media_type: ContentMediaType::new("application/octet-stream").expect("媒体类型有效"),
            encryption_mode: ContentEncryptionMode::ServerSide,
            scan_state: ContentScanState::Pending,
            lifecycle_state: ContentLifecycleState::Uploading,
            expires_at: None,
            created_at: UtcMillis::new(1).expect("时间有效"),
            deleted_at: None,
        })
        .expect("内容有效")
    }

    fn digest(payload: &[u8]) -> Sha256Digest {
        Sha256Digest::from_bytes(Sha256::digest(payload).into())
    }

    fn length(payload: &[u8]) -> ContentByteLength {
        ContentByteLength::new(u64::try_from(payload.len()).expect("长度可转换")).expect("正文非空")
    }
}
