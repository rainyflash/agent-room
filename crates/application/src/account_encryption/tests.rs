use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use agent_room_domain::{ids::PrincipalId, time::UtcMillis};
use uuid::Uuid;

use super::{
    AccountEncryptionDependencies, AccountEncryptionFailure, AccountEncryptionService,
    EncryptionKeyBytes, RESETS_PER_HOUR,
};
use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AccountEncryptionKeyRepository, AccountEncryptionKeySealer, Clock,
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
    allowed: Mutex<Vec<String>>,
    failing: Mutex<bool>,
}

impl MatrixCrossSigningResetGateway for Synapse {
    fn allow_cross_signing_replacement<'a>(
        &'a self,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        self.allowed
            .lock()
            .unwrap()
            .push(user_id.as_str().to_owned());
        let result = if *self.failing.lock().unwrap() {
            Err(MatrixFailure::new(
                MatrixOperation::AllowCrossSigningReplacement,
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

#[tokio::test]
async fn 重建签名身份前请_synapse_开豁免_一小时最多三次() {
    let harness = harness();
    let rainy = person("rainy");

    for _ in 0..RESETS_PER_HOUR {
        harness.service.allow_reset(&rainy).await.unwrap();
        *harness.clock.0.lock().unwrap() += 60_000;
    }
    assert_eq!(
        *harness.synapse.allowed.lock().unwrap(),
        vec!["@rainy:matrix.agent-room.localhost".to_owned(); RESETS_PER_HOUR]
    );
    let Err(AccountEncryptionFailure::RateLimited { retry_at }) =
        harness.service.allow_reset(&rainy).await
    else {
        panic!("第四次要被挡下");
    };
    assert_eq!(retry_at.value(), NOW + 3_600_000, "最早那次满一小时后再来");
    assert_eq!(
        harness.synapse.allowed.lock().unwrap().len(),
        RESETS_PER_HOUR
    );

    // 别人不受影响；满一小时后又能重建。
    harness.service.allow_reset(&person("other")).await.unwrap();
    *harness.clock.0.lock().unwrap() = NOW + 3_600_000;
    harness.service.allow_reset(&rainy).await.unwrap();
}

#[tokio::test]
async fn synapse_不可用时说暂时不可用() {
    let harness = harness();
    *harness.synapse.failing.lock().unwrap() = true;
    assert_eq!(
        harness.service.allow_reset(&person("rainy")).await,
        Err(AccountEncryptionFailure::Unavailable)
    );
}
