use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use agent_room_domain::{ids::PrincipalId, time::UtcMillis};
use uuid::Uuid;

use super::{
    EmailReminderSettings, ReminderDependencies, ReminderFailure, ReminderService,
    ReminderSettings, UnsubscribeFailure, UnsubscribeStanding,
};
use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, PortFuture, ReminderAccount, ReminderSettingsRepository, ReminderUnsubscribeTokens,
    },
};

const NOW: i64 = 1_790_000_000_000;

/// 假的库：`rows` 是存过的开关和改的时间，`gone` 是正在删、已经删了的账户。
#[derive(Default)]
struct Settings {
    rows: Mutex<HashMap<PrincipalId, (bool, UtcMillis)>>,
    gone: Mutex<Vec<PrincipalId>>,
    unavailable: Mutex<bool>,
}

impl Settings {
    fn failure(&self) -> Option<RepositoryError> {
        (*self.unavailable.lock().unwrap())
            .then(|| RepositoryError::new("test.reminders", RepositoryErrorKind::Unavailable))
    }
}

impl ReminderSettingsRepository for Settings {
    fn find_reminder_account(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<ReminderAccount>> {
        let result = self.failure().map_or_else(
            || {
                Ok(if self.gone.lock().unwrap().contains(&principal_id) {
                    ReminderAccount::Gone
                } else {
                    ReminderAccount::Active {
                        email_enabled: self
                            .rows
                            .lock()
                            .unwrap()
                            .get(&principal_id)
                            .is_none_or(|(enabled, _)| *enabled),
                    }
                })
            },
            Err,
        );
        Box::pin(async move { result })
    }

    fn set_email_reminders(
        &self,
        principal_id: PrincipalId,
        enabled: bool,
        changed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        let result = self.failure().map_or_else(
            || {
                if self.gone.lock().unwrap().contains(&principal_id) {
                    return Ok(false);
                }
                let mut rows = self.rows.lock().unwrap();
                let unchanged = rows
                    .get(&principal_id)
                    .is_some_and(|(current, _)| *current == enabled);
                if !unchanged {
                    rows.insert(principal_id, (enabled, changed_at));
                }
                Ok(true)
            },
            Err,
        );
        Box::pin(async move { result })
    }
}

/// 假的令牌：`token:<账户 ID>`，别的一律不认。
struct Tokens;

impl ReminderUnsubscribeTokens for Tokens {
    fn issue(&self, principal_id: PrincipalId) -> String {
        format!("token:{}", principal_id.as_uuid())
    }

    fn verify(&self, token: &str) -> Option<PrincipalId> {
        token
            .strip_prefix("token:")
            .and_then(|id| Uuid::parse_str(id).ok())
            .map(PrincipalId::from_uuid)
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(NOW).unwrap()
    }
}

fn service(settings: &Arc<Settings>, available: bool, tokens: bool) -> ReminderService {
    ReminderService::new(ReminderDependencies {
        repository: settings.clone(),
        tokens: tokens.then(|| Arc::new(Tokens) as Arc<dyn ReminderUnsubscribeTokens>),
        clock: Arc::new(FixedClock),
        email_available: available,
    })
}

fn person() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
        matrix_user_id: "@grace:matrix.agent-room.localhost".to_owned(),
        display_name: "Grace".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: UtcMillis::new(NOW - 1_000).unwrap(),
        expires_at: UtcMillis::new(NOW + 60_000).unwrap(),
        recently_authenticated: false,
    }
}

const fn settings_view(enabled: bool, available: bool) -> ReminderSettings {
    ReminderSettings {
        email: EmailReminderSettings { enabled, available },
    }
}

#[tokio::test]
async fn 没存过的照默认开着_服务器开没开照配置() {
    let settings = Arc::new(Settings::default());
    let actor = person();
    assert_eq!(
        service(&settings, false, true).settings(&actor).await,
        Ok(settings_view(true, false))
    );
    assert_eq!(
        service(&settings, true, true).settings(&actor).await,
        Ok(settings_view(true, true))
    );
}

#[tokio::test]
async fn 关上和打开都存下_服务器没开也存() {
    let settings = Arc::new(Settings::default());
    let reminders = service(&settings, false, true);
    let actor = person();
    assert_eq!(
        reminders.set_email(&actor, false).await,
        Ok(settings_view(false, false))
    );
    assert_eq!(
        reminders.settings(&actor).await,
        Ok(settings_view(false, false))
    );
    assert_eq!(
        settings.rows.lock().unwrap().get(&actor.principal_id),
        Some(&(false, UtcMillis::new(NOW).unwrap()))
    );
    assert_eq!(
        reminders.set_email(&actor, true).await,
        Ok(settings_view(true, false))
    );
}

#[tokio::test]
async fn 账户正在删除或库不可用时说暂时不可用() {
    let settings = Arc::new(Settings::default());
    let reminders = service(&settings, true, true);
    let actor = person();
    settings.gone.lock().unwrap().push(actor.principal_id);
    assert_eq!(
        reminders.settings(&actor).await,
        Err(ReminderFailure::Unavailable)
    );
    assert_eq!(
        reminders.set_email(&actor, false).await,
        Err(ReminderFailure::Unavailable)
    );

    let other = person();
    *settings.unavailable.lock().unwrap() = true;
    assert_eq!(
        reminders.settings(&other).await,
        Err(ReminderFailure::Unavailable)
    );
}

#[tokio::test]
async fn 退订链接打开只看不改_点了才退_再点还是已经退了() {
    let settings = Arc::new(Settings::default());
    let reminders = service(&settings, true, true);
    let actor = person();
    let token = Tokens.issue(actor.principal_id);

    assert_eq!(
        reminders.unsubscribe_standing(&token).await,
        Ok(UnsubscribeStanding::Subscribed)
    );
    assert!(settings.rows.lock().unwrap().is_empty());

    assert_eq!(reminders.unsubscribe(&token).await, Ok(()));
    assert_eq!(
        reminders.unsubscribe_standing(&token).await,
        Ok(UnsubscribeStanding::Unsubscribed)
    );
    assert_eq!(reminders.unsubscribe(&token).await, Ok(()));
    // 退了以后在设置里显示关着，能再打开。
    assert_eq!(
        reminders.settings(&actor).await,
        Ok(settings_view(false, true))
    );
    assert_eq!(
        reminders.set_email(&actor, true).await,
        Ok(settings_view(true, true))
    );
}

#[tokio::test]
async fn 删掉的账户退订也算退了_什么都不写() {
    let settings = Arc::new(Settings::default());
    let reminders = service(&settings, true, true);
    let actor = person();
    settings.gone.lock().unwrap().push(actor.principal_id);
    let token = Tokens.issue(actor.principal_id);
    assert_eq!(
        reminders.unsubscribe_standing(&token).await,
        Ok(UnsubscribeStanding::Unsubscribed)
    );
    assert_eq!(reminders.unsubscribe(&token).await, Ok(()));
    assert!(settings.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 令牌不对或没配钥匙时一律不认() {
    let settings = Arc::new(Settings::default());
    let actor = person();
    let token = Tokens.issue(actor.principal_id);
    let reminders = service(&settings, true, true);
    assert_eq!(
        reminders.unsubscribe_standing("not-a-token").await,
        Err(UnsubscribeFailure::InvalidToken)
    );
    assert_eq!(
        reminders.unsubscribe("not-a-token").await,
        Err(UnsubscribeFailure::InvalidToken)
    );

    let unsigned = service(&settings, false, false);
    assert_eq!(
        unsigned.unsubscribe_standing(&token).await,
        Err(UnsubscribeFailure::InvalidToken)
    );
    assert_eq!(
        unsigned.unsubscribe(&token).await,
        Err(UnsubscribeFailure::InvalidToken)
    );
    assert!(settings.rows.lock().unwrap().is_empty());

    *settings.unavailable.lock().unwrap() = true;
    assert_eq!(
        reminders.unsubscribe(&token).await,
        Err(UnsubscribeFailure::Unavailable)
    );
}
