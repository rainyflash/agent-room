use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use agent_room_domain::{ids::PrincipalId, time::UtcMillis};
use serde_json::json;
use uuid::Uuid;

use super::{
    AccountEncryptionDependencies, AccountEncryptionFailure, AccountEncryptionService,
    EncryptionKeyBytes, RESETS_PER_HOUR,
};
use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AccountEncryptionKeyRepository, AccountEncryptionKeySealer, Clock, MatrixCrossSigningKeys,
        MatrixCrossSigningResetGateway, MatrixFailure, MatrixFailureKind, MatrixOperation,
        MatrixResult, MatrixUserId, PortFuture, SealedSecret, SecretSealingFailure,
        StoredEncryptionKey,
    },
};

const NOW: i64 = 1_790_000_000_000;

#[derive(Default)]
struct Keys {
    rows: Mutex<HashMap<PrincipalId, (StoredEncryptionKey, UtcMillis)>>,
    unavailable: Mutex<bool>,
}

impl AccountEncryptionKeyRepository for Keys {
    fn find_encryption_key(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<StoredEncryptionKey>>> {
        let result = if *self.unavailable.lock().unwrap() {
            Err(RepositoryError::new(
                "test.find",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .get(&principal_id)
                .map(|(key, _)| key.clone()))
        };
        Box::pin(async move { result })
    }

    fn put_encryption_key<'a>(
        &'a self,
        principal_id: PrincipalId,
        key: &'a StoredEncryptionKey,
        updated_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        self.rows
            .lock()
            .unwrap()
            .insert(principal_id, (key.clone(), updated_at));
        Box::pin(async { Ok(()) })
    }
}

/// 假的封存：密文前面带上主体，换个主体就打不开；`missing_key` 模拟部署没配封存密钥。
#[derive(Default)]
struct Sealer {
    missing_key: bool,
}

impl AccountEncryptionKeySealer for Sealer {
    fn seal(
        &self,
        principal_id: PrincipalId,
        plaintext: &[u8],
    ) -> Result<SealedSecret, SecretSealingFailure> {
        if self.missing_key {
            return Err(SecretSealingFailure::Unavailable);
        }
        let mut bytes = principal_id.as_uuid().as_bytes().to_vec();
        bytes.extend(plaintext.iter().map(|byte| byte ^ 0x5a));
        Ok(SealedSecret {
            key_version: 1,
            bytes,
        })
    }

    fn open(
        &self,
        principal_id: PrincipalId,
        sealed: &SealedSecret,
    ) -> Result<Vec<u8>, SecretSealingFailure> {
        if self.missing_key {
            return Err(SecretSealingFailure::Unavailable);
        }
        let (owner, ciphertext) = sealed.bytes.split_at(16);
        if owner != principal_id.as_uuid().as_bytes() {
            return Err(SecretSealingFailure::Corrupt);
        }
        Ok(ciphertext.iter().map(|byte| byte ^ 0x5a).collect())
    }
}

#[derive(Default)]
struct Synapse {
    uploaded: Mutex<Vec<(String, MatrixCrossSigningKeys)>>,
    failing: Mutex<bool>,
}

impl MatrixCrossSigningResetGateway for Synapse {
    fn replace_cross_signing_keys<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
        keys: &'a MatrixCrossSigningKeys,
    ) -> PortFuture<'a, MatrixResult<()>> {
        self.uploaded
            .lock()
            .unwrap()
            .push((user_id.as_str().to_owned(), keys.clone()));
        let result = if *self.failing.lock().unwrap() {
            Err(MatrixFailure::new(
                MatrixOperation::ReplaceCrossSigningKeys,
                MatrixFailureKind::DependencyUnavailable,
            ))
        } else {
            Ok(())
        };
        Box::pin(async move { result })
    }
}

struct SteppingClock(Mutex<i64>);

impl Clock for SteppingClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(*self.0.lock().unwrap()).unwrap()
    }
}

struct Harness {
    keys: Arc<Keys>,
    synapse: Arc<Synapse>,
    clock: Arc<SteppingClock>,
    service: AccountEncryptionService,
}

fn harness_with(sealer: Sealer) -> Harness {
    let keys = Arc::new(Keys::default());
    let synapse = Arc::new(Synapse::default());
    let clock = Arc::new(SteppingClock(Mutex::new(NOW)));
    let service = AccountEncryptionService::new(AccountEncryptionDependencies {
        repository: keys.clone(),
        sealer: Arc::new(sealer),
        matrix: synapse.clone(),
        clock: clock.clone(),
    });
    Harness {
        keys,
        synapse,
        clock,
        service,
    }
}

fn harness() -> Harness {
    harness_with(Sealer::default())
}

fn person(name: &str) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
        matrix_user_id: format!("@{name}:matrix.agent-room.localhost"),
        display_name: name.to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: UtcMillis::new(NOW - 1_000).unwrap(),
        expires_at: UtcMillis::new(NOW + 60_000).unwrap(),
        recently_authenticated: false,
    }
}

fn key(byte: u8) -> EncryptionKeyBytes {
    EncryptionKeyBytes::new(vec![byte; 32])
}

/// 设备新建的签名公钥（只有公钥和签名）。
fn signing_keys(user: &str) -> serde_json::Value {
    let key = |usage: &str| {
        json!({
            "user_id": user,
            "usage": [usage],
            "keys": { format!("ed25519:{usage}"): usage },
        })
    };
    json!({
        "master_key": key("master"),
        "self_signing_key": key("self_signing"),
        "user_signing_key": key("user_signing"),
    })
}

#[tokio::test]
async fn 存下的钥匙只交还给本人_库里是封存过的() {
    let harness = harness();
    let rainy = person("rainy");
    let other = person("other");

    assert!(
        harness.service.find(&rainy).await.unwrap().is_none(),
        "还没有钥匙"
    );
    harness
        .service
        .store(&rainy, "KEY1".to_owned(), &key(7))
        .await
        .unwrap();

    let found = harness.service.find(&rainy).await.unwrap().expect("存过了");
    assert_eq!(found.key_id, "KEY1");
    assert_eq!(found.key.expose(), [7; 32]);
    assert!(
        harness.service.find(&other).await.unwrap().is_none(),
        "别人拿不到"
    );
    let rows = harness.keys.rows.lock().unwrap();
    let (stored, updated_at) = &rows[&rainy.principal_id];
    assert_ne!(stored.sealed.bytes[16..], [7; 32], "库里不是明文");
    assert_eq!(updated_at.value(), NOW);
}

#[tokio::test]
async fn 再存一次就覆盖旧的() {
    let harness = harness();
    let rainy = person("rainy");
    harness
        .service
        .store(&rainy, "OLD".to_owned(), &key(1))
        .await
        .unwrap();
    harness
        .service
        .store(&rainy, "NEW".to_owned(), &key(2))
        .await
        .unwrap();

    let found = harness.service.find(&rainy).await.unwrap().unwrap();
    assert_eq!(found.key_id, "NEW");
    assert_eq!(found.key.expose(), [2; 32]);
}

#[tokio::test]
async fn 钥匙不是_32_字节或者_id_不对就不收() {
    let harness = harness();
    let rainy = person("rainy");
    for (key_id, bytes) in [
        ("KEY1", vec![1; 31]),
        ("KEY1", vec![1; 33]),
        ("", vec![1; 32]),
        ("bad\nid", vec![1; 32]),
    ] {
        assert_eq!(
            harness
                .service
                .store(&rainy, key_id.to_owned(), &EncryptionKeyBytes::new(bytes))
                .await,
            Err(AccountEncryptionFailure::InvalidKey)
        );
    }
    assert_eq!(
        harness
            .service
            .store(&rainy, "x".repeat(256), &key(1))
            .await,
        Err(AccountEncryptionFailure::InvalidKey)
    );
    assert!(harness.keys.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 部署没配封存密钥或数据库不可用时说暂时不可用() {
    let rainy = person("rainy");
    let unsealed = harness_with(Sealer { missing_key: true });
    assert_eq!(
        unsealed
            .service
            .store(&rainy, "KEY1".to_owned(), &key(1))
            .await,
        Err(AccountEncryptionFailure::Unavailable)
    );

    let offline = harness();
    *offline.keys.unavailable.lock().unwrap() = true;
    assert_eq!(
        offline.service.find(&rainy).await.unwrap_err(),
        AccountEncryptionFailure::Unavailable
    );
}

const RAINY: &str = "@rainy:matrix.agent-room.localhost";

#[tokio::test]
async fn 重建签名身份时替本人代传新的签名公钥_一小时最多三次() {
    let harness = harness();
    let rainy = person("rainy");

    for _ in 0..RESETS_PER_HOUR {
        harness
            .service
            .replace_cross_signing_keys(&rainy, signing_keys(RAINY))
            .await
            .unwrap();
        *harness.clock.0.lock().unwrap() += 60_000;
    }
    {
        let uploaded = harness.synapse.uploaded.lock().unwrap();
        assert_eq!(uploaded.len(), RESETS_PER_HOUR);
        assert!(uploaded.iter().all(|(user, keys)| user == RAINY
            && keys.as_json().get("master_key") == signing_keys(RAINY).get("master_key")));
    }
    let Err(AccountEncryptionFailure::RateLimited { retry_at }) = harness
        .service
        .replace_cross_signing_keys(&rainy, signing_keys(RAINY))
        .await
    else {
        panic!("第四次要被挡下");
    };
    assert_eq!(retry_at.value(), NOW + 3_600_000, "最早那次满一小时后再来");
    assert_eq!(
        harness.synapse.uploaded.lock().unwrap().len(),
        RESETS_PER_HOUR
    );

    // 别人不受影响；满一小时后又能重建。
    harness
        .service
        .replace_cross_signing_keys(
            &person("other"),
            signing_keys("@other:matrix.agent-room.localhost"),
        )
        .await
        .unwrap();
    *harness.clock.0.lock().unwrap() = NOW + 3_600_000;
    harness
        .service
        .replace_cross_signing_keys(&rainy, signing_keys(RAINY))
        .await
        .unwrap();
}

#[tokio::test]
async fn 签名公钥不是本人的_或者多带了别的东西就不传_也不占次数() {
    let harness = harness();
    let rainy = person("rainy");
    let mut with_auth = signing_keys(RAINY);
    with_auth["auth"] = json!({ "type": "m.login.dummy" });
    let mut without_master = signing_keys(RAINY);
    without_master.as_object_mut().unwrap().remove("master_key");
    for body in [
        signing_keys("@someone-else:matrix.agent-room.localhost"),
        with_auth,
        without_master,
        json!(["master_key"]),
    ] {
        assert_eq!(
            harness
                .service
                .replace_cross_signing_keys(&rainy, body)
                .await,
            Err(AccountEncryptionFailure::InvalidCrossSigningKeys)
        );
    }
    assert!(harness.synapse.uploaded.lock().unwrap().is_empty());
    for _ in 0..RESETS_PER_HOUR {
        harness
            .service
            .replace_cross_signing_keys(&rainy, signing_keys(RAINY))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn synapse_不可用时说暂时不可用() {
    let harness = harness();
    *harness.synapse.failing.lock().unwrap() = true;
    assert_eq!(
        harness
            .service
            .replace_cross_signing_keys(&person("rainy"), signing_keys(RAINY))
            .await,
        Err(AccountEncryptionFailure::Unavailable)
    );
}
