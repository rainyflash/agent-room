import { Outlet } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';

import { useRuntimeServices } from '@/app/app-services';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { DesktopRuntimeSurface } from '@/features/desktop/ui/desktop-runtime-surface';

export function DesktopRootLayout() {
  const { t } = useTranslation();
  const services = useRuntimeServices();
  if (services.runtimeMode !== 'desktop') {
    throw new Error('DesktopRootLayout requires desktop runtime services.');
  }
  return (
    <DesktopRuntimeProvider gateway={services.desktop}>
      <a className="skip-link" href="#main-content">
        {t('app.skipToContent')}
      </a>
      <Outlet />
      <DesktopRuntimeSurface />
    </DesktopRuntimeProvider>
  );
}
