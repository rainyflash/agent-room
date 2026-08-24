import { StatusMark } from '@agent-room/ui-system';
import { MessageSquareText, Radio, ShieldAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { RoomMessageSignal } from '@/features/messages/domain/message';

const MAX_VISIBLE_SIGNALS = 50;

export type MessageSignalDockProps = {
  readonly messages: readonly RoomMessageSignal[];
  readonly onRetry: () => void;
  readonly onSelectMessage: (messageId: string) => void;
  readonly selectedMessageId: string | null;
  readonly state: 'failed' | 'ready';
};

export function MessageSignalDock({
  messages,
  onRetry,
  onSelectMessage,
  selectedMessageId,
  state,
}: MessageSignalDockProps) {
  const { i18n, t } = useTranslation();
  const formatter = new Intl.DateTimeFormat(i18n.resolvedLanguage, {
    hour: '2-digit',
    minute: '2-digit',
  });
  return (
    <section aria-labelledby="message-dock-title" className="message-dock">
      <header className="message-dock__header">
        <div className="message-dock__heading">
          <Radio aria-hidden="true" />
          <div>
            <p className="eyebrow">{t('messages.dock.eyebrow')}</p>
            <h2 id="message-dock-title">{t('messages.dock.title')}</h2>
          </div>
        </div>
        <span className="message-dock__count">{messages.length}</span>
      </header>
      {state === 'failed' ? (
        <div className="message-dock__boundary" role="status">
          <ShieldAlert aria-hidden="true" />
          <p>{t('messages.dock.failed')}</p>
          <button onClick={onRetry} type="button">
            {t('messages.dock.retry')}
          </button>
        </div>
      ) : messages.length === 0 ? (
        <div className="message-dock__empty">
          <MessageSquareText aria-hidden="true" />
          <p>{t('messages.dock.empty')}</p>
        </div>
      ) : (
        <ol aria-label={t('messages.dock.listLabel')} className="message-dock__list">
          {messages.slice(0, MAX_VISIBLE_SIGNALS).map((message) => (
            <li key={message.messageId}>
              <button
                aria-pressed={selectedMessageId === message.messageId}
                className="message-signal"
                onClick={() => {
                  onSelectMessage(message.messageId);
                }}
                type="button"
              >
                <span className="message-signal__author" aria-hidden="true">
                  {initials(message.actor.displayName)}
                </span>
                <span className="message-signal__copy">
                  <span className="message-signal__meta">
                    <strong>{message.actor.displayName}</strong>
                    <time dateTime={new Date(message.serverTimestamp).toISOString()}>
                      {formatter.format(message.serverTimestamp)}
                    </time>
                  </span>
                  <span className="message-signal__title">
                    {message.preview?.title ?? t(`messages.lifecycle.${message.lifecycle}`)}
                  </span>
                  <span className="message-signal__summary">
                    {message.preview?.summary ?? t('messages.preview.unavailable')}
                  </span>
                  <span className="message-signal__footer">
                    <StatusMark label={t('messages.scope.room')} tone="network" />
                    <span>{t('messages.scope.room')}</span>
                    {message.edited ? <span>{t('messages.preview.edited')}</span> : null}
                    {message.preview?.riskFlags.length === 0 ? null : (
                      <span className="message-signal__risk">
                        {t('messages.preview.riskCount', {
                          count: message.preview?.riskFlags.length ?? 0,
                        })}
                      </span>
                    )}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

function initials(displayName: string): string {
  return [...displayName.trim()].slice(0, 2).join('').toUpperCase();
}
