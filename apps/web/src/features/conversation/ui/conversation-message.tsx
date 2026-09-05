import { Bot, Reply } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { initials } from '@/shared/ui/display-name';

export function ConversationMessage({
  message,
  parent,
  names,
  editable,
  own,
  onReply,
  time,
}: {
  readonly message: RoomMessageSignal;
  readonly parent: RoomMessageSignal | undefined;
  readonly names: ReadonlyMap<string, string>;
  readonly editable: boolean;
  readonly own: boolean;
  readonly onReply: (message: RoomMessageSignal) => void;
  readonly time: Intl.DateTimeFormat;
}) {
  const { t } = useTranslation();
  const chat = message.preview?.conversation;
  return (
    <article
      className={`conversation-message${own ? ' conversation-message--own' : ''}`}
      data-actor-kind={message.actor.kind}
    >
      <div className="conversation-message__avatar" aria-hidden="true">
        {message.actor.kind === 'agent' ? <Bot /> : initials(message.actor.displayName)}
      </div>
      <div className="conversation-message__body">
        <header>
          <strong>{message.actor.displayName}</strong>
          <span>
            {t(message.actor.kind === 'human' ? 'conversation.human' : 'conversation.agent')}
          </span>
          <time dateTime={new Date(message.serverTimestamp).toISOString()}>
            {time.format(message.serverTimestamp)}
          </time>
        </header>
        <div className="conversation-message__text">
          {message.relation === undefined ? null : (
            <blockquote>
              {parent?.lifecycle === 'active'
                ? `${parent.actor.displayName}: ${parent.preview?.summary ?? ''}`
                : t('conversation.referenced')}
            </blockquote>
          )}
          {chat?.mentions.length ? (
            <div className="conversation-message__mentions">
              {chat.mentions.map((id) => (
                <span key={id} title={id}>
                  @{names.get(id) ?? id}
                </span>
              ))}
            </div>
          ) : null}
          <p>{chat?.text}</p>
        </div>
      </div>
      <button
        className="conversation-message__reply"
        type="button"
        disabled={!editable}
        aria-label={t('conversation.reply', { name: message.actor.displayName })}
        title={t('conversation.reply', { name: message.actor.displayName })}
        onClick={() => {
          onReply(message);
        }}
      >
        <Reply aria-hidden="true" />
      </button>
    </article>
  );
}
