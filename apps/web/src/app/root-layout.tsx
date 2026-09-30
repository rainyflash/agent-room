import { ToastStack } from '@agent-room/ui-system';
import { Outlet, useLocation } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { ThisComputerBanner } from '@/features/desktop/ui/this-computer-banner';
import { MatrixVerificationInbox } from '@/features/security/ui/matrix-verification-inbox';
import { SessionProvider, useSession } from '@/features/session/ui/session-provider';
import { ConversationWorkspaceProvider } from '@/features/conversation/ui/conversation-workspace-context';
import { FrontendTelemetryObserver } from '@/features/telemetry/ui/frontend-telemetry-observer';
import { RuntimeCompatibilityProvider } from '@/features/updates/ui/runtime-compatibility-provider';
import { DesktopUpdateToast } from '@/features/updates/ui/desktop-update-toast';
import { UpdatePrompt } from '@/features/updates/ui/update-prompt';
import { InboxNotice } from '@/features/inbox/ui/inbox-notice';

export function RootLayout() {
  const pathname = useLocation({ select: (location) => location.pathname });
  const services = useAppServices();
  return (
    <DesktopRuntimeProvider gateway={services.localRuntime} telemetry={services.telemetry}>
      <WebRootLayout pathname={pathname} />
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
      {pathname === '/' ? (
        <>
          <Outlet />
          <ThisComputerBanner />
          <ToastStack label={t('toasts.label')}>
            <DesktopUpdateToast />
            <UpdatePrompt />
          </ToastStack>
        </>
      ) : (
        <WebSessionRuntime pathname={pathname} />
      )}
    </RuntimeCompatibilityProvider>
  );
}

function WebSessionRuntime({ pathname }: { readonly pathname: string }) {
  const { t } = useTranslation();
  const { session, telemetry } = useAppServices();
  return (
    <SessionProvider dependencies={session}>
      <FrontendTelemetryObserver gateway={telemetry} />
      <ConversationSessionOutlet />
      {/* “我的 Agent”页自己有“这台电脑”一节，不再重复提示。 */}
      <ThisComputerBanner hidden={pathname === '/workspace'} />
      {/* 新版本、新消息、别的设备请求核对：右下角一个提示栈，不再互相压住。 */}
      <ToastStack label={t('toasts.label')}>
        <MatrixVerificationInbox />
        <InboxNotice />
        <DesktopUpdateToast />
        <UpdatePrompt />
      </ToastStack>
    </SessionProvider>
  );
}

function ConversationSessionOutlet() {
  const { snapshot } = useSession();
  const { messagePublisher } = useAppServices();
  return (
    <ConversationWorkspaceProvider
      publisher={messagePublisher}
      scope={snapshot.context.principal?.matrixUserId ?? null}
    >
      <Outlet />
    </ConversationWorkspaceProvider>
  );
}
