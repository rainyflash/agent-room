import { Banner, Button, Details, type BannerTone } from '@agent-room/ui-system';
import { ExternalLink, RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import {
  authorizationFailureMessage,
  localConnectionNotice,
  waitingForServer,
} from '../domain/desktop-connection';
import type { BridgePhase } from '../domain/desktop-runtime';
import { useDesktopRuntimeController } from './desktop-runtime-provider';

const toneByPhase: Readonly<Partial<Record<BridgePhase, BannerTone>>> = {
  authorization_required: 'warning',
  halted: 'danger',
};

/**
 * 这台电脑的连接服务没就绪时的一条提示。启动、重连只是告知，不挡着复制；要授权或停了才给按钮。
 * 错误码收进详情。
 */
export function LocalConnectionNotice() {
  const { i18n, t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const phase = controller.snapshot?.bridge.lifecycle.phase ?? 'discovering';
  const authorization = controller.snapshot?.bridge.authorization;
  const lifecycle = controller.snapshot?.bridge.lifecycle;
  if (phase === 'ready' || phase === 'authorized') return null;
  const authorizationFailed =
    phase === 'halted' && lifecycle?.diagnosticCode === 'desktop.authorization.failed';
  const serverUnreachable = waitingForServer(lifecycle);
  const nextRetryAtUnixMs = serverUnreachable ? lifecycle?.nextRetryAtUnixMs : null;
  const action =
    phase === 'authorization_required' ? (
      authorization == null ? undefined : (
        <Button
          disabled={controller.busy !== null}
          icon={<ExternalLink aria-hidden="true" />}
          onClick={() => void controller.openAuthorization(authorization.promptId)}
          size="compact"
          tone="primary"
        >
          {t('agentInvite.runtime.authorizeAction')}
        </Button>
      )
    ) : phase === 'discovering' || phase === 'starting' ? undefined : (
      <Button
        disabled={controller.busy !== null}
        icon={<RefreshCw aria-hidden="true" />}
        onClick={() => void controller.retryBridge()}
        size="compact"
        tone="ghost"
      >
        {t('agentInvite.runtime.retryAction')}
      </Button>
    );
  return (
    <Banner action={action} className="local-connection-notice" tone={toneByPhase[phase] ?? 'info'}>
      <p>
        {t(
          authorizationFailed
            ? authorizationFailureMessage(lifecycle.lastFailureCode)
            : serverUnreachable
              ? 'agentInvite.runtime.serverUnreachable'
              : localConnectionNotice[phase],
        )}
      </p>
      {nextRetryAtUnixMs == null ? null : (
        <p>
          {t('agentInvite.runtime.nextAttempt', {
            time: new Intl.DateTimeFormat(i18n.resolvedLanguage, {
              hour: '2-digit',
              minute: '2-digit',
              second: '2-digit',
            }).format(new Date(nextRetryAtUnixMs)),
          })}
        </p>
      )}
      {(phase === 'halted' || serverUnreachable) && lifecycle?.lastFailureCode != null ? (
        <Details summary={t('connection.details')}>
          <code>{lifecycle.lastFailureCode}</code>
        </Details>
      ) : null}
    </Banner>
  );
}
