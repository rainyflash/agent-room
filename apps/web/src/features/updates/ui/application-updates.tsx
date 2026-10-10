import { Button, Details } from '@agent-room/ui-system';
import { Download, RefreshCw } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { updateProgressLabel } from '@/features/desktop/domain/update-progress';
import { manifestExpiredCode } from '@/features/desktop/domain/update-status';
import {
  defaultReleaseChannel,
  type ReleaseUpdateChannel,
  type ReleaseUpdateStatus,
} from '@/features/desktop/domain/desktop-runtime';
import { applicationVersion } from '../domain/runtime-manifest';
import {
  applyWebApplicationUpdate,
  checkWebApplicationUpdate,
} from '../adapters/browser-application-update';
import { useRuntimeCompatibility } from './runtime-compatibility-context';
import './application-about.css';

/**
 * 应用更新：桌面端选渠道、检查、安装并重启；网页端检查并重新载入。只有这一处能装更新，
 * 桌面端放在“设置 → 这台电脑”，网页端放在“设置 → 关于”。桌面端的更新由原生层定时查，
 * 这里写上次检查的时间和结果。
 */
export function ApplicationUpdates() {
  const { t, i18n } = useTranslation();
  const desktop = useOptionalDesktopRuntimeController();
  const native = desktop?.available === true;
  const runtime = useRuntimeCompatibility();
  const [channel, setChannel] = useState<ReleaseUpdateChannel>(
    defaultReleaseChannel(applicationVersion),
  );
  const [checking, setChecking] = useState(false);
  const [applying, setApplying] = useState(false);
  const [checkedAt, setCheckedAt] = useState<number | null>(null);
  const [webUpdate, setWebUpdate] = useState(false);
  const [failure, setFailure] = useState(false);
  const selectedUpdate = desktop?.update?.channel === channel ? desktop.update : null;
  // 上次检查走的是别的渠道时不说，免得把那个渠道的结果当成这个渠道的。
  const status = native && desktop.updateStatus?.channel === channel ? desktop.updateStatus : null;
  const translocated = native && desktop.snapshot?.appTranslocated === true;
  const updateAvailable = native
    ? selectedUpdate?.available === true
    : runtime.updateWaiting || webUpdate;
  const busy = checking || applying || desktop?.updateBusy != null;
  const check = async () => {
    if (busy) return;
    setChecking(true);
    setFailure(false);
    try {
      if (native) await desktop.checkUpdate(channel);
      else setWebUpdate(await checkWebApplicationUpdate());
      setCheckedAt(Date.now());
    } catch {
      setFailure(true);
    } finally {
      setChecking(false);
    }
  };
  const install = async () => {
    if (busy) return;
    setApplying(true);
    setFailure(false);
    try {
      if (native) await desktop.installUpdate();
      else await applyWebApplicationUpdate(runtime.applyUpdate);
    } catch {
      setFailure(true);
    } finally {
      setApplying(false);
    }
  };
  const checkedTime = (checkedAtUnixMs: number) => {
    const checked = new Date(checkedAtUnixMs);
    const today = checked.toDateString() === new Date().toDateString();
    return new Intl.DateTimeFormat(
      i18n.resolvedLanguage,
      today ? { timeStyle: 'short' } : { dateStyle: 'medium', timeStyle: 'short' },
    ).format(checked);
  };
  return (
    <section className="application-about__updates" aria-label={t('application.updates')}>
      <h3>{t('application.updates')}</h3>
      {native ? (
        <label>
          {t('application.channel')}
          <select
            value={channel}
            disabled={busy}
            onChange={(event) => {
              if (event.target.value === 'stable' || event.target.value === 'testing') {
                setChannel(event.target.value);
                setCheckedAt(null);
              }
            }}
          >
            <option value="stable">{t('desktop.update.channel.stable')}</option>
            <option value="testing">{t('desktop.update.channel.testing')}</option>
          </select>
        </label>
      ) : (
        <p>{t('application.webHint')}</p>
      )}
      {native && desktop.snapshot?.updatesConfigured === false ? (
        <p>{t('application.unconfigured')}</p>
      ) : (
        <>
          <div className="application-about__buttons">
            <Button
              icon={<RefreshCw aria-hidden="true" />}
              disabled={busy}
              onClick={() => void check()}
            >
              {busy
                ? applying || desktop?.updateBusy === 'installing'
                  ? native
                    ? updateProgressLabel(t, desktop.updateProgress ?? null)
                    : t('application.installing')
                  : t('application.checking')
                : t('desktop.update.check')}
            </Button>
            {updateAvailable && !translocated ? (
              <Button
                icon={<Download aria-hidden="true" />}
                disabled={busy}
                onClick={() => void install()}
              >
                {t(
                  native
                    ? selectedUpdate?.rollback
                      ? 'desktop.update.rollback'
                      : 'application.install'
                    : 'pwa.update.action',
                )}
              </Button>
            ) : null}
          </div>
          <div role="status">
            {updateAvailable ? (
              <p>
                {native
                  ? t('desktop.update.available', {
                      current: selectedUpdate?.currentVersion,
                      target: selectedUpdate?.targetVersion,
                    })
                  : t('application.webAvailable')}
              </p>
            ) : native ? (
              status === null ? null : (
                <LastCheck status={status} time={checkedTime(status.checkedAtUnixMs)} />
              )
            ) : checkedAt !== null && !failure ? (
              <p>
                {t('application.current')}{' '}
                {t('application.checkedAt', {
                  time: new Intl.DateTimeFormat(i18n.resolvedLanguage, {
                    timeStyle: 'short',
                  }).format(checkedAt),
                })}
              </p>
            ) : null}
            {translocated ? <p>{t('desktop.update.translocated')}</p> : null}
          </div>
        </>
      )}
      {failure || (native && desktop.updateFailure != null) ? (
        <div role="alert">
          <p>
            {t(
              desktop?.updateFailure?.code === 'desktop.update.draft_unsaved'
                ? 'conversation.draftUnavailable'
                : 'application.failed',
            )}
          </p>
          {desktop?.updateFailure == null ? null : (
            <Details summary={t('entry.details')}>
              <code>{desktop.updateFailure.code}</code>
            </Details>
          )}
        </div>
      ) : null}
    </section>
  );
}

/** 上次检查的结果：已是最新、清单过期（下次发版就好）、没查成（错误码收进详情）。 */
function LastCheck({
  status,
  time,
}: {
  readonly status: ReleaseUpdateStatus;
  readonly time: string;
}) {
  const { t } = useTranslation();
  if (status.failure === null) return <p>{t('application.lastCheck.current', { time })}</p>;
  if (status.failure.code === manifestExpiredCode) {
    return <p>{t('application.lastCheck.expired', { time })}</p>;
  }
  return (
    <>
      <p>{t('application.lastCheck.failed', { time })}</p>
      <Details summary={t('entry.details')}>
        <code>{status.failure.code}</code>
      </Details>
    </>
  );
}
