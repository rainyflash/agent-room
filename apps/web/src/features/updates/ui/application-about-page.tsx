import { useState } from 'react';
import { Download, ExternalLink, Monitor, RefreshCw, Globe2 } from 'lucide-react';
import { Button } from '@agent-room/ui-system';
import { useTranslation } from 'react-i18next';
import { AppNavigation } from '@/shared/ui/app-navigation';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { applicationVersion } from '../domain/runtime-manifest';
import { updateProgressLabel } from '@/features/desktop/domain/update-progress';
import { useRuntimeCompatibility } from './runtime-compatibility-context';
import {
  applyWebApplicationUpdate,
  checkWebApplicationUpdate,
} from '../adapters/browser-application-update';
import {
  defaultReleaseChannel,
  type ReleaseUpdateChannel,
} from '@/features/desktop/domain/desktop-runtime';
import './application-about.css';

export function ApplicationAboutPage() {
  const { t, i18n } = useTranslation();
  const desktop = useOptionalDesktopRuntimeController();
  const native = desktop?.available === true;
  const runtime = useRuntimeCompatibility();
  const currentVersion = native
    ? (desktop.snapshot?.currentVersion ?? desktop.update?.currentVersion ?? null)
    : applicationVersion;
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
    <main className="application-about" id="main-content">
      <AppNavigation />
      <section className="application-about__card">
        <header>
          <img src="/agent-room-mark.svg" alt="" />
          <div>
            <p>{t('application.about')}</p>
            <h1>{t('app.name')}</h1>
            <span>
              {native ? <Monitor aria-hidden="true" /> : <Globe2 aria-hidden="true" />}
              {t(native ? 'application.desktop' : 'application.web')}
            </span>
          </div>
        </header>
        <dl>
          <div>
            <dt>{t('application.version')}</dt>
            <dd>
              {currentVersion === null ? t('application.versionUnavailable') : `v${currentVersion}`}
            </dd>
          </div>
          <div>
            <dt>{t('application.build')}</dt>
            <dd>{t(import.meta.env.DEV ? 'application.development' : 'application.release')}</dd>
          </div>
          {checkedAt === null ? null : (
            <div>
              <dt>{t('application.lastChecked')}</dt>
              <dd>
                {new Intl.DateTimeFormat(i18n.resolvedLanguage, {
                  dateStyle: 'short',
                  timeStyle: 'short',
                }).format(checkedAt)}
              </dd>
            </div>
          )}
        </dl>
        <section className="application-about__updates" aria-label={t('application.updates')}>
          <h2>{t('application.updates')}</h2>
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
                  <p>{t('application.current')}</p>
                ) : null}
              </div>
            </>
          )}
          {failure || (native && desktop.updateFailure != null) ? (
            <div role="alert">
              <p>{t('application.failed')}</p>
              {desktop?.updateFailure == null ? null : (
                <details>
                  <summary>{t('halls.details')}</summary>
                  <code>{desktop.updateFailure.code}</code>
                </details>
              )}
            </div>
          ) : null}
        </section>
        <footer>
          <a
            href="https://github.com/rainyflash/agent-room/releases"
            target="_blank"
            rel="noreferrer"
          >
            {t('application.releaseNotes')}
            <ExternalLink aria-hidden="true" />
          </a>
        </footer>
      </section>
    </main>
  );
}
