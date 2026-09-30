import { Button } from '@agent-room/ui-system';
import { LogIn } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { readAutomationGrantDraft } from '@/features/automation/adapters/automation-grant-draft';
import type { AuthenticationCallbackFailure } from '@/features/session/domain/authentication-callback';
import type { ControlPlaneGateway, WebSession } from '@/features/session/domain/session';
import { EntryCard } from '@/shared/ui/entry-shell';

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
    <EntryCard
      actions={
        <>
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
        </>
      }
      icon={<LogIn />}
      title={t('connection.authenticationRecovery.title')}
    >
      <p className="entry-card__lede" role="alert">
        {t(`connection.authenticationRecovery.${reason}`)}
      </p>
      {principal === null ? null : (
        <p className="entry-card__lede">{t('connection.authenticationRecovery.sessionRetained')}</p>
      )}
      {saved?.ok === false ? <p role="alert">{t('automation.draft.failed')}</p> : null}
      {failed ? <p role="alert">{t('connection.authenticationRecovery.retryFailed')}</p> : null}
    </EntryCard>
  );
}
