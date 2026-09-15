import { Link } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';
import { applicationVersion } from '../domain/runtime-manifest';
import { useOptionalDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import './application-about.css';

export function ApplicationVersionLink() {
  const { t } = useTranslation();
  const desktop = useOptionalDesktopRuntimeController();
  const version = desktop?.available
    ? (desktop.snapshot?.currentVersion ?? desktop.update?.currentVersion)
    : applicationVersion;
  return (
    <Link
      to="/about"
      className="application-version-link"
      aria-label={
        version === undefined ? t('application.about') : t('application.aboutVersion', { version })
      }
      title={t('application.about')}
    >
      {version === undefined ? t('application.about') : `v${version}`}
    </Link>
  );
}
