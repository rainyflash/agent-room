import { Banner, Button, Segmented } from '@agent-room/ui-system';
import { BellOff, BellRing, LayoutGrid, List } from 'lucide-react';
import { useEffect, useId, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { webNotificationsActive } from '@/features/inbox/adapters/web-notifications';
import { useWebNotificationState } from '@/features/inbox/ui/web-notifications-provider';
import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { useOptionalAccountPreferences } from '@/features/preferences/ui/account-preferences-provider';
import { LanguageControl } from '@/features/preferences/ui/language-control';

const HOUR_MS = 3_600_000;

/** 通用：语言、进房间时的视图，和“提醒”一组（网页通知、暂停提醒）。原来散在顶栏、房间和收件箱里。 */
export function GeneralSettings() {
  const { t } = useTranslation();
  const preferences = useOptionalAccountPreferences();
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
      <QuietSetting />
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

/** 暂停提醒一小时：新消息照样进收件箱，只是不弹出来。 */
function QuietSetting() {
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
    ? t('settings.general.quietHint')
    : indefinite
      ? t('settings.general.quietOn', { time: '—' })
      : t('settings.general.quietOn', {
          time: new Intl.DateTimeFormat(i18n.resolvedLanguage, { timeStyle: 'short' }).format(
            until,
          ),
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
