import { Outlet, useLocation } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { DesktopRuntimeSurface } from '@/features/desktop/ui/desktop-runtime-surface';
import { MatrixVerificationInbox } from '@/features/security/ui/matrix-verification-inbox';
import { SessionProvider } from '@/features/session/ui/session-provider';
import { FrontendTelemetryObserver } from '@/features/telemetry/ui/frontend-telemetry-observer';
import { RuntimeCompatibilityProvider } from '@/features/updates/ui/runtime-compatibility-provider';
import { UpdatePrompt } from '@/features/updates/ui/update-prompt';

export function RootLayout() {
  const { t } = useTranslation();
  const pathname = useLocation({ select: (location) => location.pathname });
  return (
    <RuntimeCompatibilityProvider>
      <a className="skip-link" href="#main-content">
        {t('app.skipToContent')}
      </a>
      {pathname === '/' ? <Outlet /> : <SessionRuntime />}
      <UpdatePrompt />
    </RuntimeCompatibilityProvider>
  );
}

function SessionRuntime() {
  const { desktop, session, telemetry } = useAppServices();
  return (
    <SessionProvider dependencies={session}>
      <FrontendTelemetryObserver gateway={telemetry} />
      <Outlet />
      <DesktopRuntimeSurface gateway={desktop} telemetry={telemetry} />
      <MatrixVerificationInbox />
    </SessionProvider>
  );
}
