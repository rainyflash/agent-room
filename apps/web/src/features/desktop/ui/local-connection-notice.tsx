import { Button } from '@agent-room/ui-system';
import { AlertTriangle, ExternalLink, RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { localConnectionNotice } from '../domain/desktop-connection';
import { useDesktopRuntimeController } from './desktop-runtime-provider';

export function LocalConnectionNotice() {
  const { t } = useTranslation();
  const controller = useDesktopRuntimeController();
  const phase = controller.snapshot?.bridge.lifecycle.phase ?? 'discovering';
  const authorization = controller.snapshot?.bridge.authorization;
  const lifecycle = controller.snapshot?.bridge.lifecycle;
  const authorizationFailed =
    phase === 'halted' && lifecycle?.diagnosticCode === 'desktop.authorization.failed';
  if (phase === 'ready' || phase === 'authorized') return null;
  return (
    <section className="agent-invite__notice" data-phase={phase} role="status">
      <AlertTriangle aria-hidden="true" />
      <div>
        <p>
          {t(
            authorizationFailed
              ? 'desktop.authorization.failedDescription'
              : localConnectionNotice[phase],
          )}
        </p>
        {phase === 'halted' && lifecycle?.lastFailureCode != null ? (
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
