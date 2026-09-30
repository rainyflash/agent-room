import { Link } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';
import { applicationVersion } from '../domain/runtime-manifest';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import './application-about.css';

/** 首页和房间菜单里的版本号，点开是“设置 → 关于”。桌面端查到新版本时换成“有新版本”，点开去“设置 → 这台电脑”安装。 */
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
      params={{ section: update ? 'this-computer' : 'about' }}
      to="/settings/$section"
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
