import { Banner, Button, Segmented } from '@agent-room/ui-system';
import { BellOff, BellRing, LayoutGrid, List } from 'lucide-react';
import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useId, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useOptionalAppServices } from '@/app/app-services';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { webNotificationsActive } from '@/features/inbox/adapters/web-notifications';
import { useWebNotificationState } from '@/features/inbox/ui/web-notifications-provider';
import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { useOptionalAccountPreferences } from '@/features/preferences/ui/account-preferences-provider';
import { LanguageControl } from '@/features/preferences/ui/language-control';
import {
  reminderSettingsQueryKey,
  useReminderSettings,
} from '@/features/reminders/data/reminder-settings-queries';
import type { ReminderSettingsGateway } from '@/features/reminders/domain/reminder-settings';
import { useOptionalSession } from '@/features/session/ui/session-provider';

const HOUR_MS = 3_600_000;

/**
 * 通用：语言、进房间时的视图，和“提醒”一组（网页通知、邮件提醒、暂停提醒）。原来散在顶栏、房间和
 * 收件箱里。
 */
export function GeneralSettings() {
  const { t } = useTranslation();
  const preferences = useOptionalAccountPreferences();
  const reminders = useOptionalAppServices()?.reminders;
  const principalId = useOptionalSession()?.snapshot.context.principal?.principalId ?? null;
  return (
    <div className="settings-rows">
      <div className="settings-row">
        <div>
          <strong>{t('settings.general.language')}</strong>
          <p>{t('settings.general.languageHint')}</p>
        </div>
        <LanguageControl />
      </div>
      {preferences === null ? null : (
        <div className="settings-row">
          <div>
            <strong>{t('settings.general.roomView')}</strong>
            <p>{t('settings.general.roomViewHint')}</p>
          </div>
          <Segmented
            label={t('settings.general.roomView')}
            onChange={(next) => {
              preferences.setLobbyView(next);
            }}
            options={[
              {
                value: 'scene',
                label: t('settings.general.roomView.scene'),
                icon: <LayoutGrid />,
              },
              { value: 'list', label: t('settings.general.roomView.list'), icon: <List /> },
            ]}
            value={preferences.snapshot.values.lobbyView}
          />
        </div>
      )}
      <h3 className="settings-group">{t('settings.general.reminders')}</h3>
      <WebNotificationSetting />
      {reminders === undefined ? (
        <QuietSetting email={false} />
      ) : (
        <EmailAndQuietSettings gateway={reminders} principalId={principalId} />
      )}
    </div>
  );
}

/**
 * 网页通知（specs/redesign/reminders.md 4R-2）：只在这个浏览器里，打开时浏览器还没问过就先问。
 * 桌面端没有这一行：应用自己会弹系统通知。
 */
function WebNotificationSetting() {
  const { t } = useTranslation();
  const desktop = useOptionalDesktopRuntimeController();
  const { notifications, state } = useWebNotificationState();
  const [busy, setBusy] = useState(false);
  const label = useId();
  const hint = useId();
  if (desktop?.available === true) return null;
  const blocked = state.permission === 'denied' || state.permission === 'unsupported';
  const status =
    state.permission === 'denied'
      ? t('settings.general.webNotificationsDenied')
      : state.permission === 'unsupported'
        ? t('settings.general.webNotificationsUnsupported')
        : t('settings.general.webNotificationsHint');
  return (
    <div className="settings-row">
      <div>
        <strong id={label}>{t('settings.general.webNotifications')}</strong>
        <p className={blocked ? 'settings-row__blocked' : undefined} id={hint}>
          {status}
        </p>
      </div>
      <input
        aria-describedby={hint}
        aria-labelledby={label}
        checked={webNotificationsActive(state)}
        className="settings-switch"
        disabled={blocked || busy}
        onChange={(event) => {
          setBusy(true);
          void notifications.setWanted(event.target.checked).finally(() => {
            setBusy(false);
          });
        }}
        role="switch"
        type="checkbox"
      />
    </div>
  );
}

/** 读一次本人的提醒设置：这台服务器没开邮件提醒、或者读不出来时，不显示邮件那一行，暂停也不提邮件。 */
function EmailAndQuietSettings({
  gateway,
  principalId,
}: {
  readonly gateway: ReminderSettingsGateway;
  readonly principalId: string | null;
}) {
  const settings = useReminderSettings(gateway, principalId);
  const email =
    settings.data?.ok === true && settings.data.value.email.available
      ? settings.data.value.email
      : null;
  return (
    <>
      {email === null ? null : (
        <EmailReminderSetting enabled={email.enabled} gateway={gateway} principalId={principalId} />
      )}
      <QuietSetting email={email !== null} />
    </>
  );
}

/** 邮件提醒（specs/redesign/reminders.md 4R-3）：跟着账户走，存在控制面；改了马上存。 */
function EmailReminderSetting({
  enabled,
  gateway,
  principalId,
}: {
  readonly enabled: boolean;
  readonly gateway: ReminderSettingsGateway;
  readonly principalId: string | null;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const label = useId();
  const hint = useId();
  return (
    <>
      <div className="settings-row">
        <div>
          <strong id={label}>{t('settings.general.emailReminders')}</strong>
          <p id={hint}>{t('settings.general.emailRemindersHint')}</p>
        </div>
        <input
          aria-describedby={hint}
          aria-labelledby={label}
          checked={enabled}
          className="settings-switch"
          disabled={busy}
          onChange={(event) => {
            setBusy(true);
            setFailed(false);
            void gateway
              .setEmail(event.target.checked)
              .then((result) => {
                if (result.ok) {
                  queryClient.setQueryData(reminderSettingsQueryKey(principalId), result);
                } else {
                  setFailed(true);
                }
              })
              .finally(() => {
                setBusy(false);
              });
          }}
          role="switch"
          type="checkbox"
        />
      </div>
      {failed ? (
        <Banner role="alert" tone="danger">
          {t('settings.general.quietFailed')}
        </Banner>
      ) : null}
    </>
  );
}

/** 暂停提醒一小时：新消息照样进收件箱，只是不弹出来；开着邮件提醒时，也不发提醒邮件。 */
function QuietSetting({ email }: { readonly email: boolean }) {
  const { i18n, t } = useTranslation();
  const personal = usePersonalWorkspace();
  const [now, setNow] = useState(() => Date.now());
  const [failed, setFailed] = useState(false);
  const until = personal?.snapshot.index.doNotDisturbUntil ?? 0;
  const indefinite = personal?.snapshot.index.doNotDisturbUntil === null;
  const quiet = indefinite || until > now;
  // 到点自动恢复，界面跟着变回来。
  useEffect(() => {
    if (until <= Date.now()) return undefined;
    const timer = setTimeout(
      () => {
        setNow(Date.now());
      },
      until - Date.now() + 10,
    );
    return () => {
      clearTimeout(timer);
    };
  }, [until]);
  if (personal === null) return null;
  const status = !quiet
    ? t(email ? 'settings.general.quietHintEmail' : 'settings.general.quietHint')
    : t(email ? 'settings.general.quietOnEmail' : 'settings.general.quietOn', {
        time: indefinite
          ? '—'
          : new Intl.DateTimeFormat(i18n.resolvedLanguage, { timeStyle: 'short' }).format(until),
      });
  return (
    <>
      <div className="settings-row">
        <div>
          <strong>{t('settings.general.quiet')}</strong>
          <p>{status}</p>
        </div>
        <Button
          aria-pressed={quiet}
          icon={quiet ? <BellRing aria-hidden="true" /> : <BellOff aria-hidden="true" />}
          onClick={() => {
            const result = personal.change({
              kind: 'dnd',
              id: 'global',
              value: quiet ? 0 : Date.now() + HOUR_MS,
            });
            setFailed(!result.ok);
            setNow(Date.now());
          }}
          size="compact"
          tone="ghost"
        >
          {t(quiet ? 'settings.general.quietResume' : 'settings.general.quietHour')}
        </Button>
      </div>
      {failed ? (
        <Banner role="alert" tone="danger">
          {t('settings.general.quietFailed')}
        </Banner>
      ) : null}
    </>
  );
}
