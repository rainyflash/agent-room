import { ToastStack } from '@agent-room/ui-system';
import { Outlet, useLocation } from '@tanstack/react-router';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { DesktopRuntimeProvider } from '@/features/desktop/ui/desktop-runtime-provider';
import { ThisComputerBanner } from '@/features/desktop/ui/this-computer-banner';
import { SessionProvider, useSession } from '@/features/session/ui/session-provider';
import { MatrixConnectionToast } from '@/features/session/ui/matrix-connection-toast';
import { ConversationWorkspaceProvider } from '@/features/conversation/ui/conversation-workspace-context';
import { FrontendTelemetryObserver } from '@/features/telemetry/ui/frontend-telemetry-observer';
import { RuntimeCompatibilityProvider } from '@/features/updates/ui/runtime-compatibility-provider';
import { DesktopUpdateToast } from '@/features/updates/ui/desktop-update-toast';
import { UpdatePrompt } from '@/features/updates/ui/update-prompt';
import { InboxNotice } from '@/features/inbox/ui/inbox-notice';
import { AgentKnockNotice } from '@/features/private-rooms/ui/agent-knock-notice';

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
      {outsideSession(pathname) ? (
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
      {/* 新版本、新消息、消息没连上、有 Agent 敲门：右下角一个提示栈，不再互相压住。 */}
      <ToastStack label={t('toasts.label')}>
        <MatrixConnectionToast hidden={explainsMatrixConnection(pathname)} />
        <AgentKnockNotice pathname={pathname} />
        <InboxNotice />
        <DesktopUpdateToast />
        <UpdatePrompt />
      </ToastStack>
    </SessionProvider>
  );
}

/**
 * 首页、围观页和隐私说明不进登录会话：没登录的人打开它们，不会触发清理登录状态（那会中止别的请求、
 * 清空查询缓存）。它们要知道登录了没有，自己问服务器。
 */
function outsideSession(pathname: string): boolean {
  return (
    pathname === '/' ||
    pathname === '/privacy' ||
    pathname === '/watch' ||
    pathname.startsWith('/watch/')
  );
}

/** 这几页没连上消息时自己会说（连接页、安全设置、房间页），提示栈里就不再重复。 */
function explainsMatrixConnection(pathname: string): boolean {
  return (
    pathname === '/connect' || pathname === '/settings/security' || pathname.startsWith('/lobby/')
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
