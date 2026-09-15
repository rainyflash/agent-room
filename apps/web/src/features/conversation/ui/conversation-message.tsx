import { Reply } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { AgentPortrait } from '@/features/lobby/ui/room-illustration';
import { initials } from '@/shared/ui/display-name';
import type { AgentDelivery } from '../domain/message-delivery';
import { MessageAttachment } from './message-attachment';

export function ConversationMessage({
  message,
  parent,
  names,
  editable,
  own,
  onReply,
  time,
  delivery = [],
}: {
  readonly message: RoomMessageSignal;
  readonly parent: RoomMessageSignal | undefined;
  readonly names: ReadonlyMap<string, string>;
  readonly editable: boolean;
  readonly own: boolean;
  readonly onReply: (message: RoomMessageSignal) => void;
  readonly time: Intl.DateTimeFormat;
  readonly delivery?: readonly AgentDelivery[];
}) {
  const { t } = useTranslation();
  const chat = message.preview?.conversation;
  return (
    <article
      className={`conversation-message${own ? ' conversation-message--own' : ''}`}
      data-actor-kind={message.actor.kind}
    >
      <div className="conversation-message__avatar" aria-hidden="true">
        {message.actor.kind === 'agent' ? (
          <AgentPortrait id={message.actor.agentId} />
        ) : (
          initials(message.actor.displayName)
        )}
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
          {chat?.attachmentName !== undefined && chat.text === chat.attachmentName ? null : (
            <p>{chat?.text}</p>
          )}
          {chat?.attachmentName !== undefined && message.content !== null ? (
            <MessageAttachment
              content={message.content}
              roomId={message.roomId}
              name={chat.attachmentName}
            />
          ) : null}
        </div>
        {own ? (
          <div
            className="conversation-message__delivery"
            aria-label={t('conversation.delivery.title')}
          >
            <span>{t('conversation.delivery.sent')}</span>
            {delivery.length === 0 ? (
              <span>{t('conversation.delivery.unconfirmed')}</span>
            ) : (
              delivery.map((item) => (
                <span key={item.agentId} data-stage={item.stage}>
                  {item.name} · {t(`conversation.delivery.${item.stage}`)}
                </span>
              ))
            )}
          </div>
        ) : null}
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
