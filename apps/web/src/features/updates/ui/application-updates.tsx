import { Button, Details } from '@agent-room/ui-system';
import { Download, RefreshCw } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { updateProgressLabel } from '@/features/desktop/domain/update-progress';
import {
  defaultReleaseChannel,
  type ReleaseUpdateChannel,
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
 * 桌面端放在“设置 → 这台电脑”，网页端放在“设置 → 关于”。
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
            {updateAvailable ? (
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
            ) : checkedAt !== null && !failure && (!native || selectedUpdate !== null) ? (
              <p>
                {t('application.current')}{' '}
                {t('application.checkedAt', {
                  time: new Intl.DateTimeFormat(i18n.resolvedLanguage, {
                    timeStyle: 'short',
                  }).format(checkedAt),
                })}
              </p>
            ) : null}
          </div>
        </>
      )}
      {failure || (native && desktop.updateFailure != null) ? (
        <div role="alert">
          <p>{t('application.failed')}</p>
          {desktop?.updateFailure == null ? null : (
            <Details summary={t('connection.details')}>
              <code>{desktop.updateFailure.code}</code>
            </Details>
          )}
        </div>
      ) : null}
    </section>
  );
}
