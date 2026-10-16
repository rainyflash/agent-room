//! 桌面端登录着要电脑授权、Bridge 拿它注册这台电脑（`specs/desktop-sign-in/design.md`）。

use std::sync::{Arc, atomic::AtomicI64};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    devices::{
        ComputerGrantService, ComputerGrantUseCases, DeviceAuthorizationFailureKind,
        DeviceAuthorizationService, RegisterDeviceWithGrant,
    },
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        ComputerGrantRecord, ComputerGrantStore, DeviceSessionRegistration, DeviceSignature,
        PortFuture, PrincipalAccount, SecretDigest, SecretValue, StoredDeviceSession,
    },
};
use agent_room_domain::{
    devices::{Device, DevicePlatform, DevicePublicSigningKey},
    identity::Principal,
    ids::PrincipalId,
    time::UtcMillis,
};
use uuid::Uuid;

use super::{
    DAY, MemoryDeviceStore, MemoryRefreshToken, NOW, SteppingClock, attempt_id, refresh_at,
    service_with_clock, time,
};

const MINUTE: i64 = 60 * 1_000;

impl ComputerGrantStore for MemoryDeviceStore {
    fn insert_grant<'a>(
        &'a self,
        grant: &'a ComputerGrantRecord,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("测试设备仓储锁不得中毒");
            state
                .grants
                .retain(|_, existing| existing.expires_at > grant.created_at);
            state.grants.insert(grant.grant_digest, grant.clone());
            Ok(())
        })
    }

    fn find_grant<'a>(
        &'a self,
        grant_digest: &'a SecretDigest,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<ComputerGrantRecord>>> {
        Box::pin(async move {
            let state = self.state.lock().expect("测试设备仓储锁不得中毒");
            Ok(state
                .grants
                .get(grant_digest)
                .filter(|grant| grant.expires_at > now)
                .cloned())
        })
    }

    fn register_with_grant<'a>(
        &'a self,
        grant: &'a ComputerGrantRecord,
        device: &'a Device,
        session: &'a DeviceSessionRegistration,
    ) -> PortFuture<'a, RepositoryResult<StoredDeviceSession>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("测试设备仓储锁不得中毒");
            let consumed = state.grants.remove(&grant.grant_digest).filter(|stored| {
                stored.principal_id == grant.principal_id && stored.expires_at > session.issued_at
            });
            if consumed.is_none() {
                return Err(RepositoryError::new(
                    "device.register_with_grant",
                    RepositoryErrorKind::NotFound,
                ));
            }
            let stored = StoredDeviceSession {
                account: PrincipalAccount {
                    principal: Principal::new(grant.principal_id),
                    matrix_user_id: "@rainy:matrix.example.test".to_owned(),
                    display_name: "Rainy".to_owned(),
                    avatar_content_id: None,
                    locale: "zh-CN".to_owned(),
                },
                device: device.clone(),
                family: session.family.clone(),
                access_token_expires_at: session.access_token_expires_at,
            };
            state.access_token_digest = Some(session.access_token_digest);
            state
                .refresh_tokens
                .insert(session.refresh_token_digest, MemoryRefreshToken::default());
            state.session = Some(stored.clone());
            Ok(stored)
        })
    }
}

struct Grants {
    devices: DeviceAuthorizationService,
    grants: ComputerGrantService,
    store: Arc<MemoryDeviceStore>,
    clock: Arc<SteppingClock>,
    verifier: Arc<super::ToggleProofVerifier>,
}

fn grants() -> Grants {
    let clock = Arc::new(SteppingClock(AtomicI64::new(NOW)));
    let (devices, store, _, verifier, _) = service_with_clock(true, clock.clone());
    let grants = devices.computer_grants(store.clone());
    Grants {
        devices,
        grants,
        store,
        clock,
        verifier,
    }
}

fn principal(authenticated_at: i64) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: PrincipalId::from_uuid(Uuid::from_u128(7)),
        matrix_user_id: "@rainy:matrix.example.test".to_owned(),
        display_name: "Rainy".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(authenticated_at),
        expires_at: time(NOW + 30 * DAY),
        recently_authenticated: false,
    }
}

fn register(grant: &SecretValue) -> RegisterDeviceWithGrant {
    RegisterDeviceWithGrant {
        grant: grant.clone(),
        label: "开发工作站".to_owned(),
        platform: DevicePlatform::Windows,
        public_signing_key: DevicePublicSigningKey::new(vec![3; 32]).expect("公钥有效"),
        possession_signature: DeviceSignature::new(vec![4; 64]).expect("签名长度有效"),
    }
}

#[tokio::test]
async fn 电脑授权注册这台电脑_一年从那份登录认证的时刻算() {
    let Grants {
        devices,
        grants,
        store,
        clock,
        ..
    } = grants();
    let issued = grants
        .issue_computer_grant(&principal(NOW - 300 * DAY))
        .await
        .expect("登录着能要到电脑授权");
    assert_eq!(issued.expires_at, time(NOW + 10 * MINUTE));

    let credentials = grants
        .register_device_with_grant(register(&issued.grant))
        .await
        .expect("拿电脑授权注册成功");
    assert_eq!(credentials.refresh_token_expires_at, time(NOW + 30 * DAY));
    assert_eq!(
        credentials.device.account.principal.id(),
        PrincipalId::from_uuid(Uuid::from_u128(7)),
        "注册到授权里记的那个账户"
    );
    {
        let state = store.state.lock().expect("测试设备仓储锁不得中毒");
        let family = &state.session.as_ref().expect("会话存在").family;
        assert_eq!(family.authenticated_at(), Some(time(NOW - 300 * DAY)));
    }

    let device_id = credentials.device.device_id;
    let mut refresh_token = credentials.refresh_token;
    for (day, expected) in [(20, 50), (40, 65)] {
        clock.set(NOW + day * DAY);
        let renewed = refresh_at(
            &devices,
            device_id,
            &refresh_token,
            Some(attempt_id()),
            time(NOW + day * DAY),
        )
        .await
        .expect("一直在用就一直能续");
        assert_eq!(
            renewed.refresh_token_expires_at,
            time(NOW + expected * DAY),
            "第 {day} 天换令牌后的到期时间"
        );
        refresh_token = renewed.refresh_token;
    }

    clock.set(NOW + 65 * DAY);
    let capped = refresh_at(
        &devices,
        device_id,
        &refresh_token,
        Some(attempt_id()),
        time(NOW + 65 * DAY),
    )
    .await
    .expect_err("那份登录满一年，这台电脑也跟着到期");
    assert_eq!(capped.kind(), DeviceAuthorizationFailureKind::InvalidToken);
}

#[tokio::test]
async fn 电脑授权只能用一次() {
    let Grants { grants, .. } = grants();
    let issued = grants
        .issue_computer_grant(&principal(NOW))
        .await
        .expect("登录着能要到电脑授权");
    grants
        .register_device_with_grant(register(&issued.grant))
        .await
        .expect("第一次注册成功");

    let reused = grants
        .register_device_with_grant(register(&issued.grant))
        .await
        .expect_err("同一份授权不能再注册一次");
    assert_eq!(
        reused.kind(),
        DeviceAuthorizationFailureKind::InvalidAuthorization
    );
}

#[tokio::test]
async fn 电脑授权十分钟后不收() {
    let Grants { grants, clock, .. } = grants();
    let issued = grants
        .issue_computer_grant(&principal(NOW))
        .await
        .expect("登录着能要到电脑授权");

    clock.set(NOW + 10 * MINUTE);
    let expired = grants
        .register_device_with_grant(register(&issued.grant))
        .await
        .expect_err("过了十分钟不收");
    assert_eq!(
        expired.kind(),
        DeviceAuthorizationFailureKind::InvalidAuthorization
    );
}

#[tokio::test]
async fn 持有证明不对不注册_授权也不算用掉() {
    let Grants {
        grants, verifier, ..
    } = grants();
    let issued = grants
        .issue_computer_grant(&principal(NOW))
        .await
        .expect("登录着能要到电脑授权");

    verifier.0.store(false, std::sync::atomic::Ordering::SeqCst);
    let rejected = grants
        .register_device_with_grant(register(&issued.grant))
        .await
        .expect_err("签名对不上不注册");
    assert_eq!(
        rejected.kind(),
        DeviceAuthorizationFailureKind::InvalidProof
    );

    verifier.0.store(true, std::sync::atomic::Ordering::SeqCst);
    grants
        .register_device_with_grant(register(&issued.grant))
        .await
        .expect("签名对上以后照样能用");
}

#[tokio::test]
async fn 满一年的登录换不出这台电脑() {
    let Grants { grants, .. } = grants();
    let issued = grants
        .issue_computer_grant(&principal(NOW - 365 * DAY))
        .await
        .expect("要授权时不卡，注册时卡");

    let capped = grants
        .register_device_with_grant(register(&issued.grant))
        .await
        .expect_err("那份登录已经满一年");
    assert_eq!(
        capped.kind(),
        DeviceAuthorizationFailureKind::InvalidAuthorization
    );
}
