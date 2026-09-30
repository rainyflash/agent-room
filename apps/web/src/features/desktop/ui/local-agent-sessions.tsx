import { Details, Spinner } from '@agent-room/ui-system';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { HostSessionDiagnostics } from '../domain/desktop-runtime';
import type { DesktopRuntimeController } from './use-desktop-runtime';

type Observation =
  | { readonly kind: 'loading' }
  | { readonly kind: 'failed'; readonly code: string }
  | { readonly kind: 'ready'; readonly sessions: readonly HostSessionDiagnostics[] };

/** 与桌面诊断一致：最近 35 秒内取过信才算正在看消息。 */
const READING_WINDOW_MS = 35_000;
const POLL_MS = 5_000;

/**
 * 这台电脑上接进来的 Agent：名字、在不在房间里、这会儿有没有在看消息。读不到时如实说，
 * 不当成“一个都没有”；错误码收进详情。
 */
export function LocalAgentSessions({
  readHostSessions,
}: Pick<DesktopRuntimeController, 'readHostSessions'>) {
  const { t } = useTranslation();
  const [observation, setObservation] = useState<Observation>({ kind: 'loading' });

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
      timer = setTimeout(() => void poll(), POLL_MS);
    };
    void poll();
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, [readHostSessions]);

  return (
    <section aria-labelledby="local-agent-sessions-title" className="local-agent-sessions">
      <h3 id="local-agent-sessions-title">{t('thisComputer.agents.title')}</h3>
      {observation.kind === 'loading' ? (
        <p className="this-computer__muted">
          <Spinner />
          {t('thisComputer.agents.loading')}
        </p>
      ) : null}
      {observation.kind === 'failed' ? (
        <div role="status">
          <p>{t('thisComputer.agents.failure')}</p>
          <Details summary={t('connection.details')}>
            <code>{observation.code}</code>
          </Details>
        </div>
      ) : null}
      {observation.kind === 'ready' && observation.sessions.length === 0 ? (
        <p className="this-computer__muted">{t('thisComputer.agents.empty')}</p>
      ) : null}
      {observation.kind === 'ready' && observation.sessions.length > 0 ? (
        <ul>
          {observation.sessions.map((entry) => (
            <li data-state={entry.session.state} key={entry.session.sessionId}>
              <strong>{entry.displayName}</strong>
              <span className="local-agent-sessions__state">
                {t(`desktop.hosts.session.${entry.session.state}`)}
              </span>
              {entry.session.state === 'ready' ? (
                <span className="this-computer__muted">
                  {t(
                    entry.lastInboxReadAgoMs !== null &&
                      entry.lastInboxReadAgoMs < READING_WINDOW_MS
                      ? 'thisComputer.agents.reading'
                      : 'thisComputer.agents.notReading',
                  )}
                </span>
              ) : null}
              {entry.session.errorCode === null ? null : (
                <Details summary={t('connection.details')}>
                  <code>{entry.session.errorCode}</code>
                </Details>
              )}
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}
