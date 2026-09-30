import { Link } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';
import { applicationVersion } from '../domain/runtime-manifest';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import './application-about.css';

/** 顶栏上的版本号，点开是“关于与更新”。桌面端查到新版本时换成“有新版本”，更新只在那一页装。 */
export function ApplicationVersionLink() {
  const { t } = useTranslation();
  const desktop = useOptionalDesktopRuntimeController();
  const version = desktop?.available
    ? (desktop.snapshot?.currentVersion ?? desktop.update?.currentVersion)
    : applicationVersion;
  const update = desktop?.available === true && desktop.update?.available === true;
  const label = update
    ? t('application.updateReadyVersion', { version: desktop.update.targetVersion })
    : version === undefined
      ? t('application.about')
      : t('application.aboutVersion', { version });
  return (
    <Link
      to="/about"
      className="application-version-link"
      data-update={update ? 'true' : undefined}
      aria-label={label}
      title={label}
    >
      {update
        ? t('application.updateReady')
        : version === undefined
          ? t('application.about')
          : `v${version}`}
    </Link>
  );
}
