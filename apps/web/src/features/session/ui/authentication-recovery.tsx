import { Button } from '@agent-room/ui-system';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { readAutomationGrantDraft } from '@/features/automation/adapters/automation-grant-draft';
import type { AuthenticationCallbackFailure } from '@/features/session/domain/authentication-callback';
import type { ControlPlaneGateway, WebSession } from '@/features/session/domain/session';

export function AuthenticationRecovery({
  gateway,
  principal,
  reason,
}: {
  readonly gateway: ControlPlaneGateway;
  readonly principal: WebSession | null;
  readonly reason: AuthenticationCallbackFailure;
}) {
  const { t } = useTranslation();
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const saved = principal === null ? null : readAutomationGrantDraft(principal.principalId);
  const returnPath = saved?.ok === true ? (saved.value?.returnPath ?? '/rooms') : '/rooms';

  const retry = async (): Promise<void> => {
    setPending(true);
    setFailed(false);
    try {
      const result = await gateway.beginAuthentication(returnPath);
      if (!result.ok) {
        setFailed(true);
        setPending(false);
      }
    } catch {
      setFailed(true);
      setPending(false);
    }
  };

  return (
    <main className="connection-workspace" id="main-content">
      <section
        className="connection-workspace__stage"
        aria-labelledby="authentication-recovery-title"
      >
        <h1 id="authentication-recovery-title">{t('connection.authenticationRecovery.title')}</h1>
        <p className="connection-workspace__lede" role="alert">
          {t(`connection.authenticationRecovery.${reason}`)}
        </p>
        {principal === null ? null : (
          <p>{t('connection.authenticationRecovery.sessionRetained')}</p>
        )}
        {saved?.ok === false ? <p role="alert">{t('automation.draft.failed')}</p> : null}
        {failed ? <p role="alert">{t('connection.authenticationRecovery.retryFailed')}</p> : null}
        <div className="connection-actions">
          <Button disabled={pending} onClick={() => void retry()} size="large" tone="primary">
            {t(
              pending
                ? 'connection.authenticationRecovery.pending'
                : 'connection.authenticationRecovery.retry',
            )}
          </Button>
          {principal === null ? null : (
            <Button
              disabled={pending}
              onClick={() => {
                window.location.assign('/rooms');
              }}
              size="large"
              tone="ghost"
            >
              {t('connection.authenticationRecovery.return')}
            </Button>
          )}
        </div>
      </section>
    </main>
  );
}
