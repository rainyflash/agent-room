use std::{env, time::Duration};

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

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("缺少测试环境变量 {name}"))
}
