import { Button } from '@agent-room/ui-system';
import { AtSign, ChevronDown, ChevronUp, MessageCircle, Reply, Send, X } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  conversationMessages,
  type ConversationParticipant,
} from '@/features/conversation/domain/conversation';
import { useConversationComposer } from '@/features/conversation/ui/use-conversation-composer';
import type { MessageSubmissionIdFactory } from '@/features/messages/adapters/browser-submission-id-factory';
import type { MessagePublisher } from '@/features/messages/domain/publication';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { useRuntimeCompatibility } from '@/features/updates/ui/runtime-compatibility-context';
import './conversation-panel.css';

const emptyParticipants: readonly ConversationParticipant[] = [];

export type ConversationPanelProps = {
  readonly messages: readonly RoomMessageSignal[];
  readonly participants?: readonly ConversationParticipant[];
  readonly publisher: MessagePublisher;
  readonly roomId: string;
  readonly roomName: string;
  readonly writesAllowed?: boolean;
  readonly state: 'ready' | 'loading' | 'failed';
  readonly submissionIds?: MessageSubmissionIdFactory;
};

export function ConversationPanel({
  messages,
  participants = emptyParticipants,
  publisher,
  roomId,
  roomName,
  writesAllowed = true,
  state,
  submissionIds,
}: ConversationPanelProps) {
  const { t, i18n } = useTranslation();
  const runtime = useRuntimeCompatibility();
  const [expanded, setExpanded] = useState(true);
  const input = useRef<HTMLTextAreaElement>(null);
  const composer = useConversationComposer(publisher, roomId, submissionIds);
  const timeline = useMemo(() => conversationMessages(messages), [messages]);
  const timelineElement = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const latestEvent = timeline.at(-1)?.matrixEventId;
  useEffect(() => {
    const element = timelineElement.current;
    if (following.current && element !== null) element.scrollTop = element.scrollHeight;
  }, [latestEvent, expanded]);
  const names = useMemo(
    () =>
      new Map([
        ...messages.map(
          (message) => [message.actor.matrixUserId, message.actor.displayName] as const,
        ),
        ...participants.map(
          (participant) => [participant.matrixUserId, participant.displayName] as const,
        ),
      ]),
    [messages, participants],
  );
  const { publication } = composer;
  const pending = publication.matches('unknown') || publication.matches('acceptedBindingPending');
  const submitting = publication.matches('publishing') || publication.matches('reconciling');
  const failed = publication.matches('failed');
  const unavailable = publication.matches('identityUnavailable');
  const canSend =
    writesAllowed &&
    runtime.writes.allowed &&
    state === 'ready' &&
    composer.editable &&
    composer.valid;
  const time = new Intl.DateTimeFormat(i18n.resolvedLanguage, {
    hour: '2-digit',
    minute: '2-digit',
  });

  return (
    <section
      className={`conversation-panel${expanded ? '' : ' conversation-panel--collapsed'}`}
      aria-label={t('conversation.title')}
    >
      <header className="conversation-panel__header">
        <MessageCircle aria-hidden="true" />
        <div>
          <h2>{t('conversation.title')}</h2>
          <span>{roomName}</span>
        </div>
        <button
          type="button"
          aria-expanded={expanded}
          aria-label={t(expanded ? 'conversation.collapse' : 'conversation.open')}
          onClick={() => {
            setExpanded(!expanded);
          }}
        >
          {expanded ? <ChevronDown aria-hidden="true" /> : <ChevronUp aria-hidden="true" />}
        </button>
      </header>
      {expanded ? (
        <>
          <div
            className="conversation-panel__timeline"
            ref={timelineElement}
            onScroll={(event) => {
              const element = event.currentTarget;
              following.current =
                element.scrollHeight - element.scrollTop - element.clientHeight < 48;
            }}
            role="log"
            aria-label={t('conversation.title')}
            aria-live="polite"
            aria-relevant="additions text"
          >
            {state === 'loading' ? (
              <p>{t('conversation.loading')}</p>
            ) : state === 'failed' ? (
              <p role="alert">{t('conversation.unavailable')}</p>
            ) : timeline.length === 0 ? (
              <p className="conversation-panel__empty">{t('conversation.empty')}</p>
            ) : null}
            {timeline.map((message) => {
              const chat = message.preview?.conversation;
              const parent =
                message.relation === undefined
                  ? null
                  : messages.find(
                      (candidate) => candidate.messageId === message.relation?.targetMessageId,
                    );
              return (
                <article
                  className="conversation-message"
                  data-actor-kind={message.actor.kind}
                  key={message.messageId}
                >
                  <header>
                    <strong>{message.actor.displayName}</strong>
                    <span>
                      {t(
                        message.actor.kind === 'human'
                          ? 'conversation.human'
                          : 'conversation.agent',
                      )}
                    </span>
                    <time dateTime={new Date(message.serverTimestamp).toISOString()}>
                      {time.format(message.serverTimestamp)}
                    </time>
                  </header>
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
                  <button
                    type="button"
                    disabled={!composer.editable}
                    aria-label={t('conversation.reply', { name: message.actor.displayName })}
                    onClick={() => {
                      composer.respond(message);
                      input.current?.focus();
                    }}
                  >
                    <Reply aria-hidden="true" />
                    {t('conversation.reply', { name: message.actor.displayName })}
                  </button>
                </article>
              );
            })}
          </div>
          <form
            className="conversation-panel__composer"
            onSubmit={(event) => {
              event.preventDefault();
              if (canSend) composer.submit();
            }}
          >
            {composer.reply === null ? null : (
              <div className="conversation-panel__reply">
                <Reply aria-hidden="true" />
                <span>
                  {t('conversation.replying', { name: composer.reply.actor.displayName })}
                </span>
                <button
                  type="button"
                  disabled={!composer.editable}
                  aria-label={t('conversation.cancelReply')}
                  onClick={composer.cancelReply}
                >
                  <X aria-hidden="true" />
                </button>
              </div>
            )}
            {composer.mentions.length === 0 ? null : (
              <div className="conversation-panel__mentions">
                {composer.mentions.map((id) => (
                  <button
                    type="button"
                    key={id}
                    disabled={!composer.editable}
                    aria-label={t('conversation.removeMention', { name: names.get(id) ?? id })}
                    onClick={() => {
                      composer.removeMention(id);
                    }}
                  >
                    @{names.get(id) ?? id}
                    <X aria-hidden="true" />
                  </button>
                ))}
              </div>
            )}
            <label className="sr-only" htmlFor={`chat-${roomId}`}>
              {t('conversation.input')}
            </label>
            <textarea
              id={`chat-${roomId}`}
              ref={input}
              rows={2}
              maxLength={8_000}
              placeholder={t('conversation.placeholder')}
              value={composer.text}
              disabled={!composer.editable || !runtime.writes.allowed || !writesAllowed}
              onChange={(event) => {
                composer.changeText(event.target.value);
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
                  event.preventDefault();
                  if (canSend) composer.submit();
                }
              }}
            />
            <div className="conversation-panel__tools">
              <label>
                <AtSign aria-hidden="true" />
                <span className="sr-only">{t('conversation.mention')}</span>
                <select
                  aria-label={t('conversation.mention')}
                  value=""
                  disabled={!composer.editable || composer.mentions.length >= 8}
                  onChange={(event) => {
                    if (event.target.value) {
                      composer.mention(event.target.value);
                      input.current?.focus();
                    }
                  }}
                >
                  <option value="">{t('conversation.mention')}</option>
                  {participants
                    .filter((participant) => !composer.mentions.includes(participant.matrixUserId))
                    .map((participant) => (
                      <option key={participant.matrixUserId} value={participant.matrixUserId}>
                        {participant.displayName}
                      </option>
                    ))}
                </select>
              </label>
              <span>{t('conversation.count', { count: Array.from(composer.text).length })}</span>
              <Button
                type="submit"
                disabled={!canSend}
                icon={<Send aria-hidden="true" />}
                size="compact"
              >
                {t(submitting ? 'conversation.sending' : 'conversation.send')}
              </Button>
            </div>
            {composer.text.length > 0 && !composer.valid ? (
              <p role="alert">{t('conversation.invalid')}</p>
            ) : null}
            {publication.matches('published') ? (
              <p role="status">{t('conversation.sent')}</p>
            ) : null}
            {pending ? (
              <div role="status">
                <p>{t('conversation.pending')}</p>
                <Button onClick={composer.reconcile} size="compact">
                  {t('conversation.check')}
                </Button>
              </div>
            ) : null}
            {failed ? (
              <div role="alert">
                <p>{t('conversation.failed')}</p>
                {publication.context.failure?.retryable ? (
                  <Button onClick={composer.retry} size="compact">
                    {t('conversation.retry')}
                  </Button>
                ) : null}
                {publication.can({ type: 'CLOSE' }) ? (
                  <Button onClick={composer.edit} size="compact">
                    {t('conversation.edit')}
                  </Button>
                ) : null}
              </div>
            ) : null}
            {unavailable ? (
              <div role="alert">
                <p>{t('conversation.unavailable')}</p>
                <Button onClick={composer.retryIdentity} size="compact">
                  {t('conversation.retry')}
                </Button>
              </div>
            ) : null}
          </form>
          <details className="conversation-panel__availability">
            <summary>{t('conversation.details')}</summary>
            <p>{t('conversation.runtime')}</p>
            <p>{t('conversation.help')}</p>
          </details>
        </>
      ) : null}
    </section>
  );
}
