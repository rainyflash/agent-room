import { Button } from '@agent-room/ui-system';
import { Bot } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { AgentInviteDialog } from './agent-invite-dialog';
import type { DesktopRuntimeController } from './use-desktop-runtime';
import type { HostSessionDiagnostics } from '../domain/desktop-runtime';

type Observation =
  | { readonly kind: 'loading' }
  | { readonly kind: 'failed'; readonly code: string }
  | { readonly kind: 'ready'; readonly sessions: readonly HostSessionDiagnostics[] };

export function HostSessionOnboarding({
  readHostSessions,
}: Pick<DesktopRuntimeController, 'readHostSessions'>) {
  const { t } = useTranslation();
  const [observation, setObservation] = useState<Observation>({ kind: 'loading' });
  const [inviteOpen, setInviteOpen] = useState(false);

  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      const result = await readHostSessions();
      if (disposed) return;
      setObservation(
        result.ok
          ? { kind: 'ready', sessions: result.value }
          : { kind: 'failed', code: result.error.code },
      );
      timer = setTimeout(() => void poll(), 5_000);
    };
    void poll();
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, [readHostSessions]);

  return (
    <div className="host-onboarding">
      <h3>{t('desktop.hosts.onboarding.title')}</h3>
      <p>{t('desktop.hosts.onboarding.description')}</p>
      <Button
        icon={<Bot aria-hidden="true" />}
        onClick={() => {
          setInviteOpen(true);
        }}
        size="compact"
        tone="primary"
      >
        {t('agentInvite.open')}
      </Button>
      {inviteOpen ? (
        <AgentInviteDialog
          downloadUrl={null}
          onClose={() => {
            setInviteOpen(false);
          }}
          room={null}
        />
      ) : null}
      {observation.kind === 'loading' ? <p>{t('desktop.hosts.onboarding.loading')}</p> : null}
      {observation.kind === 'failed' ? (
        <p role="status">
          {t('desktop.hosts.onboarding.failure')} <code>{observation.code}</code>
        </p>
      ) : null}
      {observation.kind === 'ready' && observation.sessions.length === 0 ? (
        <p>{t('desktop.hosts.onboarding.empty')}</p>
      ) : null}
      {observation.kind === 'ready' && observation.sessions.length > 0 ? (
        <ul>
          {observation.sessions.map((entry) => (
            <li key={entry.session.sessionId}>
              <div>
                <strong>{entry.displayName}</strong>
                <span>{t(`desktop.hosts.session.${entry.session.state}`)}</span>
              </div>
              <p>
                {t(
                  entry.lastInboxReadAgoMs !== null && entry.lastInboxReadAgoMs < 35_000
                    ? 'desktop.hosts.session.polling'
                    : 'desktop.hosts.session.notPolling',
                )}
              </p>
              <p>
                {t(
                  entry.lastMessageReceivedAgoMs === null
                    ? 'desktop.hosts.session.noReceipt'
                    : 'desktop.hosts.session.received',
                )}{' '}
                ·{' '}
                {t(
                  entry.lastMessageSentAgoMs === null
                    ? 'desktop.hosts.session.noSend'
                    : 'desktop.hosts.session.sent',
                )}
              </p>
              {entry.session.errorCode === null ? null : <code>{entry.session.errorCode}</code>}
            </li>
          ))}
        </ul>
      ) : null}
      <p className="host-onboarding__note">{t('desktop.hosts.onboarding.note')}</p>
    </div>
  );
}
