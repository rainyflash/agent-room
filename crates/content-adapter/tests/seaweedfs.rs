use std::{
    env,
    sync::Arc,
    time::{Duration, Instant},
};

use agent_room_application::ports::{
    ContentByteStream, ContentStorageKeyFactory, ObjectStoreFailureKind, PrivateContentObjectStore,
    SecretValue,
};
use agent_room_content_adapter::{
    S3ContentStoreConfig, S3PrivateContentObjectStore, SecureContentStorageKeyFactory,
};
use agent_room_domain::{
    content::{
        ContentByteLength, ContentEncryptionMode, ContentLifecycleState, ContentMediaType,
        ContentObject, ContentObjectFields, ContentScanState, ContentStorageKey, Sha256Digest,
    },
    ids::{ContentId, PrincipalId},
    time::UtcMillis,
};
use futures_util::{StreamExt, stream};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const CAPACITY_PAYLOAD_BYTES: usize = 25 * 1_024 * 1_024;
const CAPACITY_CONCURRENCY: usize = 4;

#[tokio::test]
#[ignore = "需要通过 tools/object_store.py 启动真实 SeaweedFS"]
async fn 私有对象可幂等上传流式读取并删除() {
    let store = store();
    let payload = b"Agent Room private content";
    let content = content(payload, None);

    let first = store
        .put(&content, body(payload))
        .await
        .expect("首次上传成功");
    let repeated = store
        .put(&content, body(payload))
        .await
        .expect("相同声明重复上传保持幂等");
    assert_eq!(first, repeated);

    let opened = store.open(&content).await.expect("对象可读取");
    assert_eq!(opened.reported_digest, Some(content.digest()));
    assert_eq!(opened.reported_byte_length, Some(content.byte_length()));
    assert_eq!(collect(opened.body).await, payload);

    store.delete(&content).await.expect("删除成功");
    let failure = store.open(&content).await.expect_err("删除后不可读取");
    assert_eq!(failure.kind(), ObjectStoreFailureKind::NotFound);
}

#[tokio::test]
#[ignore = "需要通过 tools/object_store.py 启动真实 SeaweedFS"]
async fn 条件写入不会覆盖同键的既有对象() {
    let store = store();
    let original_payload = b"first immutable payload";
    let original = content(original_payload, None);
    let conflicting = content(
        b"second conflicting payload",
        Some(original.storage_key().clone()),
    );

    store
        .put(&original, body(original_payload))
        .await
        .expect("首次上传成功");
    let failure = store
        .put(&conflicting, body(b"second conflicting payload"))
        .await
        .expect_err("同键冲突不得覆盖既有对象");
    assert_eq!(failure.kind(), ObjectStoreFailureKind::CorruptMetadata);

    let opened = store.open(&original).await.expect("原始对象仍可读取");
    assert_eq!(collect(opened.body).await, original_payload);
    store.delete(&original).await.expect("清理测试对象成功");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "需要通过 tools/object_store.py 启动真实 SeaweedFS"]
async fn 四路二十五兆字节对象并发上传下载达到容量预算() {
    let payload = Arc::new(
        (0..CAPACITY_PAYLOAD_BYTES)
            .map(|index| u8::try_from(index % 251).expect("字节夹具有效"))
            .collect::<Vec<_>>(),
    );
    let expected_digest = Sha256Digest::from_bytes(Sha256::digest(payload.as_slice()).into());
    let started = Instant::now();
    let mut tasks = tokio::task::JoinSet::new();

    for _ in 0..CAPACITY_CONCURRENCY {
        let store = store();
        let payload = Arc::clone(&payload);
        tasks.spawn(async move {
            let content = content(payload.as_slice(), None);
            let upload_started = Instant::now();
            let receipt = store
                .put(&content, body(payload.as_slice()))
                .await
                .expect("25 MiB 对象上传成功");
            let upload_milliseconds = upload_started.elapsed().as_secs_f64() * 1_000.0;

            let download_started = Instant::now();
            let opened = store.open(&content).await.expect("25 MiB 对象可读取");
            let bytes = collect(opened.body).await;
            let download_milliseconds = download_started.elapsed().as_secs_f64() * 1_000.0;

            assert_eq!(receipt.byte_length.value(), CAPACITY_PAYLOAD_BYTES as u64);
            assert_eq!(receipt.digest, content.digest());
            assert_eq!(bytes.len(), CAPACITY_PAYLOAD_BYTES);
            assert_eq!(
                Sha256Digest::from_bytes(Sha256::digest(&bytes).into()),
                content.digest()
            );
            store.delete(&content).await.expect("容量对象可清理");
            (upload_milliseconds, download_milliseconds)
        });
    }

    let mut upload_samples = Vec::with_capacity(CAPACITY_CONCURRENCY);
    let mut download_samples = Vec::with_capacity(CAPACITY_CONCURRENCY);
    while let Some(result) = tasks.join_next().await {
        let (upload, download) = result.expect("对象容量任务不能崩溃");
        upload_samples.push(upload);
        download_samples.push(download);
    }
    let total_milliseconds = started.elapsed().as_secs_f64() * 1_000.0;
    let upload_p95 = percentile(&upload_samples, 95, 100);
    let download_p95 = percentile(&download_samples, 95, 100);

    assert_eq!(upload_samples.len(), CAPACITY_CONCURRENCY);
    assert_eq!(
        expected_digest,
        Sha256Digest::from_bytes(Sha256::digest(payload.as_slice()).into())
    );
    assert!(
        total_milliseconds <= 180_000.0,
        "并发往返超过 180 秒：{total_milliseconds}"
    );
    assert!(
        upload_p95 <= 120_000.0,
        "上传 P95 超过 120 秒：{upload_p95}"
    );
    assert!(
        download_p95 <= 60_000.0,
        "下载 P95 超过 60 秒：{download_p95}"
    );
    println!(
        "CAPACITY_CONTENT_OBSERVATION={{\"attachmentBytes\":{CAPACITY_PAYLOAD_BYTES},\"concurrency\":{CAPACITY_CONCURRENCY},\"totalMilliseconds\":{total_milliseconds:.3},\"uploadP95Milliseconds\":{upload_p95:.3},\"downloadP95Milliseconds\":{download_p95:.3}}}"
    );
}

fn store() -> S3PrivateContentObjectStore {
    let configuration = S3ContentStoreConfig::new(
        required("AGENT_ROOM_TEST_S3_ENDPOINT"),
        required("AGENT_ROOM_TEST_S3_BUCKET"),
        required("AGENT_ROOM_TEST_S3_REGION"),
        SecretValue::new(required("AGENT_ROOM_TEST_S3_ACCESS_KEY")).expect("访问密钥有效"),
        SecretValue::new(required("AGENT_ROOM_TEST_S3_SECRET_KEY")).expect("秘密密钥有效"),
        Duration::from_secs(15),
    )
    .expect("对象存储测试配置有效");
    S3PrivateContentObjectStore::new(&configuration)
}

fn content(payload: &[u8], storage_key: Option<ContentStorageKey>) -> ContentObject {
    let content_id = ContentId::from_uuid(Uuid::now_v7());
    let storage_key = storage_key.unwrap_or_else(|| {
        SecureContentStorageKeyFactory
            .generate(content_id)
            .expect("对象键可生成")
    });
    ContentObject::begin_upload(ContentObjectFields {
        id: content_id,
        owner_principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
        storage_key,
        digest: Sha256Digest::from_bytes(Sha256::digest(payload).into()),
        byte_length: ContentByteLength::new(u64::try_from(payload.len()).expect("长度可转换"))
            .expect("内容长度有效"),
        media_type: ContentMediaType::new("text/plain").expect("媒体类型有效"),
        encryption_mode: ContentEncryptionMode::ServerSide,
        scan_state: ContentScanState::Pending,
        lifecycle_state: ContentLifecycleState::Uploading,
        expires_at: None,
        created_at: UtcMillis::new(1_000).expect("时间有效"),
        deleted_at: None,
    })
    .expect("测试内容有效")
}

fn body(payload: &[u8]) -> ContentByteStream {
    let midpoint = payload.len() / 2;
    let chunks = [payload[..midpoint].to_vec(), payload[midpoint..].to_vec()];
    Box::pin(stream::iter(chunks.map(Ok)))
}

async fn collect(mut body: ContentByteStream) -> Vec<u8> {
    let mut collected = Vec::new();
    while let Some(chunk) = body.next().await {
        collected.extend(chunk.expect("对象流读取成功"));
    }
    collected
}

fn percentile(values: &[f64], numerator: usize, denominator: usize) -> f64 {
    assert!(!values.is_empty());
    assert!(numerator > 0 && numerator <= denominator);
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    let rank = ordered
        .len()
        .saturating_mul(numerator)
        .div_ceil(denominator);
    ordered[rank.saturating_sub(1).min(ordered.len() - 1)]
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("缺少测试环境变量 {name}"))
}
