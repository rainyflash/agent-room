import { Link } from '@tanstack/react-router';
import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AppNavigation } from '@/shared/ui/app-navigation';
import { useAppServices } from '@/app/app-services';
import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { WorkspaceSyncStatus } from '@/features/personal-workspace/ui/workspace-sync-status';
import { useInbox } from './inbox-provider';
import type { InboxItem } from '../domain/inbox';
import './inbox.css';

export function InboxPage() {
  const { t, i18n } = useTranslation();
  const inbox = useInbox();
  const personal = usePersonalWorkspace();
  const { handoffs } = useAppServices();
  const [filter, setFilter] = useState('unread');
  const [limit, setLimit] = useState(100);
  const [feedback, setFeedback] = useState<'personal.unavailable' | 'inbox.revokeFailed' | null>(
    null,
  );
  const [busy, setBusy] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now);
  const items = inbox?.snapshot.items;
  const visible = useMemo(
    () =>
      items?.filter(
        (item) => filter === 'all' || (filter === 'unread' ? !item.read : item.kind === filter),
      ) ?? [],
    [items, filter],
  );
  const time = useMemo(
    () =>
      new Intl.DateTimeFormat(i18n.resolvedLanguage, { dateStyle: 'short', timeStyle: 'short' }),
    [i18n.resolvedLanguage],
  );
  const quietUntil = personal?.snapshot.index.doNotDisturbUntil ?? 0;
  const quiet = personal?.snapshot.index.doNotDisturbUntil === null || quietUntil > now;
  useEffect(() => {
    if (quietUntil <= Date.now()) return;
    const timer = setTimeout(
      () => {
        setNow(Date.now());
      },
      quietUntil - Date.now() + 10,
    );
    return () => {
      clearTimeout(timer);
    };
  }, [quietUntil]);
  function reviewed(item: InboxItem): void {
    const result = personal?.change({ kind: 'acknowledged', id: item.id, value: item.timestamp });
    if (!result?.ok) setFeedback('personal.unavailable');
  }
  return (
    <>
      <AppNavigation active="inbox" />
      <main id="main-content" className="personal-inbox">
        <header>
          <div>
            <h1>{t('inbox.title')}</h1>
            <p>{t('inbox.description')}</p>
          </div>
          <button
            type="button"
            onClick={() => {
              inbox?.refresh();
              setNow(Date.now());
            }}
          >
            {t('inbox.refresh')}
          </button>
        </header>
        <div className="personal-inbox__tools">
          <label>
            {t('inbox.filter')}
            <select
              value={filter}
              onChange={(event) => {
                setFilter(event.target.value);
              }}
            >
              {(['unread', 'all', 'mention', 'reply', 'direct', 'handoff'] as const).map((kind) => (
                <option key={kind} value={kind}>
                  {t(`inbox.${kind}`)}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            aria-pressed={quiet}
            onClick={() => {
              const result = personal?.change({
                kind: 'dnd',
                id: 'global',
                value: quiet ? 0 : Date.now() + 3600000,
              });
              if (!result?.ok) setFeedback('personal.unavailable');
              setNow(Date.now());
            }}
          >
            {t(quiet ? 'inbox.resumeNotifications' : 'inbox.quietHour')}
          </button>
          <WorkspaceSyncStatus />
          <button
            type="button"
            disabled={!visible.some((item) => !item.read)}
            onClick={() => {
              const result = personal?.changeMany(
                visible
                  .slice(0, limit)
                  .filter((item) => !item.read)
                  .map((item) => ({ kind: 'acknowledged', id: item.id, value: item.timestamp })),
              );
              if (!result?.ok) setFeedback('personal.unavailable');
            }}
          >
            {t('inbox.markPageRead')}
          </button>
        </div>
        {inbox?.snapshot.status === 'failed' ? <p role="alert">{t('inbox.failed')}</p> : null}
        {inbox?.snapshot.status === 'loading' ? <p role="status">{t('inbox.loading')}</p> : null}
        {inbox?.snapshot.limited ? <p>{t('inbox.limited')}</p> : null}
        {(inbox?.snapshot.unavailableRooms ?? 0) > 0 ? (
          <p>{t('inbox.partial', { count: inbox?.snapshot.unavailableRooms })}</p>
        ) : null}
        {feedback ? <p role="status">{t(feedback)}</p> : null}
        {inbox?.snapshot.status === 'ready' && visible.length === 0 ? (
          <div className="personal-inbox__empty">
            <h2>{t('inbox.empty')}</h2>
            <p>{t('inbox.scope')}</p>
          </div>
        ) : null}
        <ul className="personal-inbox__list">
          {visible.slice(0, limit).map((item) => (
            <li key={item.id} data-read={item.read}>
              <div className="personal-inbox__entry">
                <div>
                  <span>
                    {t(`inbox.${item.kind}`)} · {item.room.name}
                    {item.muted ? ` · ${t('inbox.muted')}` : ''}
                  </span>
                  <h2>{item.sender}</h2>
                </div>
                <time dateTime={new Date(item.timestamp).toISOString()}>
                  {time.format(item.timestamp)}
                </time>
              </div>
              <p>
                {item.kind === 'handoff'
                  ? t(`inbox.delivery.${item.handoff?.status ?? 'queued'}`)
                  : item.text}
              </p>
              <div className="personal-inbox__actions">
                <Link
                  to="/lobby/$catalogId/instance/$roomId"
                  params={{ catalogId: item.room.catalogId, roomId: item.room.roomId }}
                  search={{
                    view: item.conversation ? 'conversation' : 'resources',
                    message: item.messageId,
                    ...(item.room.direct ? { direct: item.room.catalogId } : {}),
                  }}
                >
                  {t('inbox.open')}
                </Link>
                {!item.read ? (
                  <button
                    type="button"
                    onClick={() => {
                      reviewed(item);
                    }}
                  >
                    {t('inbox.markRead')}
                  </button>
                ) : null}
                <button
                  type="button"
                  aria-pressed={item.muted}
                  onClick={() => {
                    const result = personal?.change({
                      kind: 'mute',
                      id: item.room.roomId,
                      value: !item.muted,
                    });
                    if (!result?.ok) setFeedback('personal.unavailable');
                  }}
                >
                  {t(item.muted ? 'inbox.unmute' : 'inbox.mute')}
                </button>
                {item.handoff && item.handoff.status !== 'failed' ? (
                  <button
                    type="button"
                    disabled={busy !== null}
                    onClick={() => {
                      const handoff = item.handoff;
                      if (!handoff) return;
                      setBusy(handoff.handoffId);
                      void handoffs.revoke(handoff.handoffId).then(
                        (result) => {
                          setBusy(null);
                          if (result.ok) inbox?.refresh();
                          else setFeedback('inbox.revokeFailed');
                        },
                        () => {
                          setBusy(null);
                          setFeedback('inbox.revokeFailed');
                        },
                      );
                    }}
                  >
                    {t('inbox.revoke')}
                  </button>
                ) : null}
              </div>
            </li>
          ))}
        </ul>
        {visible.length > limit ? (
          <button
            type="button"
            onClick={() => {
              setLimit(limit + 100);
            }}
          >
            {t('inbox.more')}
          </button>
        ) : null}
        <p className="personal-inbox__scope">{t('inbox.scope')}</p>
      </main>
    </>
  );
}
