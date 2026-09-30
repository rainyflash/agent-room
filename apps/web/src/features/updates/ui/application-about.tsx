import { Link } from '@tanstack/react-router';
import { ExternalLink, Globe2, Monitor } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import { applicationVersion } from '../domain/runtime-manifest';
import { ApplicationUpdates } from './application-updates';
import './application-about.css';

/**
 * “设置 → 关于”：版本、构建、更新记录与下载。网页端在这里检查更新；桌面端的更新只在
 * “设置 → 这台电脑”一处，这里指过去。
 */
export function ApplicationAbout() {
  const { t } = useTranslation();
  const desktop = useOptionalDesktopRuntimeController();
  const native = desktop?.available === true;
  const currentVersion = native
    ? (desktop.snapshot?.currentVersion ?? desktop.update?.currentVersion ?? null)
    : applicationVersion;
  return (
    <section aria-labelledby="application-about-title" className="application-about__card">
      <header>
        <img src="/agent-room-mark.svg" alt="" />
        <div>
          <h3 id="application-about-title">{t('app.name')}</h3>
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
      </dl>
      {native ? (
        <p className="application-about__pointer">
          {t('application.desktopUpdates')}{' '}
          <Link params={{ section: 'this-computer' }} to="/settings/$section">
            {t('settings.section.thisComputer')}
          </Link>
        </p>
      ) : (
        <ApplicationUpdates />
      )}
      <footer>
        <a
          href="https://github.com/rainyflash/agent-room/releases"
          rel="noreferrer"
          target="_blank"
        >
          {t('application.releaseNotes')}
          <ExternalLink aria-hidden="true" />
        </a>
      </footer>
    </section>
  );
}
