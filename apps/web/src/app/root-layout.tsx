import { Outlet, useLocation } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { DesktopRuntimeSurface } from '@/features/desktop/ui/desktop-runtime-surface';
import { MatrixVerificationInbox } from '@/features/security/ui/matrix-verification-inbox';
import { SessionProvider } from '@/features/session/ui/session-provider';
import { FrontendTelemetryObserver } from '@/features/telemetry/ui/frontend-telemetry-observer';
import { RuntimeCompatibilityProvider } from '@/features/updates/ui/runtime-compatibility-provider';
import { UpdatePrompt } from '@/features/updates/ui/update-prompt';

export function RootLayout() {
  const pathname = useLocation({ select: (location) => location.pathname });
  const services = useAppServices();
  const desktopRuntimePlacement = pathname === '/onboarding' ? 'action-rail-safe' : 'viewport';
  return (
    <DesktopRuntimeProvider gateway={services.localRuntime} telemetry={services.telemetry}>
      <WebRootLayout pathname={pathname} />
      {pathname.includes('/instance/') && pathname.startsWith('/lobby/') ? null : (
        <DesktopRuntimeSurface placement={desktopRuntimePlacement} />
      )}
    </DesktopRuntimeProvider>
  );
}

function WebRootLayout({ pathname }: { readonly pathname: string }) {
  const { t } = useTranslation();
  return (
    <RuntimeCompatibilityProvider>
      <a className="skip-link" href="#main-content">
        {t('app.skipToContent')}
      </a>
      {pathname === '/' ? <Outlet /> : <WebSessionRuntime />}
      <UpdatePrompt />
    </RuntimeCompatibilityProvider>
  );
}

function WebSessionRuntime() {
  const { session, telemetry } = useAppServices();
  return (
    <SessionProvider dependencies={session}>
      <FrontendTelemetryObserver gateway={telemetry} />
      <Outlet />
      <MatrixVerificationInbox />
    </SessionProvider>
  );
}
