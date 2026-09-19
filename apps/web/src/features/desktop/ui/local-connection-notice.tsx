import { Button } from '@agent-room/ui-system';
import { AlertTriangle, ExternalLink, RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import {
  authorizationFailureMessage,
  localConnectionNotice,
  waitingForServer,
} from '../domain/desktop-connection';
import { useDesktopRuntimeController } from './desktop-runtime-provider';

export function LocalConnectionNotice() {
  const { i18n, t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const phase = controller.snapshot?.bridge.lifecycle.phase ?? 'discovering';
  const authorization = controller.snapshot?.bridge.authorization;
  const lifecycle = controller.snapshot?.bridge.lifecycle;
  const authorizationFailed =
    phase === 'halted' && lifecycle?.diagnosticCode === 'desktop.authorization.failed';
  const authorizationFailureDescription = authorizationFailureMessage(lifecycle?.lastFailureCode);
  const serverUnreachable = waitingForServer(lifecycle);
  const nextRetryAtUnixMs = serverUnreachable ? lifecycle?.nextRetryAtUnixMs : null;
  if (phase === 'ready' || phase === 'authorized') return null;
  return (
    <section className="agent-invite__notice" data-phase={phase} role="status">
      <AlertTriangle aria-hidden="true" />
      <div>
        <p>
          {t(
            authorizationFailed
              ? authorizationFailureDescription
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
          <details>
            <summary>{t('connection.details')}</summary>
            <code>{lifecycle.lastFailureCode}</code>
          </details>
        ) : null}
        {phase === 'authorization_required' ? (
          authorization == null ? null : (
            <Button
              disabled={controller.busy !== null}
              icon={<ExternalLink aria-hidden="true" />}
              onClick={() => void controller.openAuthorization(authorization.promptId)}
              size="compact"
              tone="network"
            >
              {t('agentInvite.runtime.authorizeAction')}
            </Button>
          )
        ) : phase === 'discovering' ? null : (
          <Button
            disabled={controller.busy !== null}
            icon={<RefreshCw aria-hidden="true" />}
            onClick={() => void controller.retryBridge()}
            size="compact"
            tone="quiet"
          >
            {t('agentInvite.runtime.retryAction')}
          </Button>
        )}
      </div>
    </section>
  );
}
