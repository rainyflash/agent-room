import { ArrowDown, ArrowUpRight, MessageCircle, Radio, UsersRound } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  conversationMessages,
  type ConversationParticipant,
} from '@/features/conversation/domain/conversation';
import { ConversationComposer } from '@/features/conversation/ui/conversation-composer';
import { ConversationMessage } from '@/features/conversation/ui/conversation-message';
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
  readonly variant?: 'room' | 'direct';
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
  variant = 'room',
}: ConversationPanelProps) {
  const { t, i18n } = useTranslation();
  const runtime = useRuntimeCompatibility();
  const reduceMotion = useReducedMotion();
  const input = useRef<HTMLTextAreaElement>(null);
  const composer = useConversationComposer(publisher, roomId, submissionIds);
  const timeline = useMemo(() => conversationMessages(messages), [messages]);
  const timelineElement = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const [unseen, setUnseen] = useState(false);
  const latestEvent = timeline.at(-1)?.matrixEventId;
  useEffect(() => {
    const element = timelineElement.current;
    if (following.current && element !== null) element.scrollTop = element.scrollHeight;
    else if (latestEvent !== undefined) setUnseen(true);
  }, [latestEvent]);
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
  const messagesById = useMemo(
    () => new Map(messages.map((message) => [message.messageId, message])),
    [messages],
  );
  const canEdit = writesAllowed && runtime.writes.allowed && composer.editable;
  const canSend = canEdit && state === 'ready' && composer.valid;
  const language = i18n.resolvedLanguage;
  const time = useMemo(
    () => new Intl.DateTimeFormat(language, { hour: '2-digit', minute: '2-digit' }),
    [language],
  );
  const date = useMemo(
    () => new Intl.DateTimeFormat(language, { month: 'long', day: 'numeric' }),
    [language],
  );
  return (
    <section className="conversation-panel" aria-label={t('conversation.title')}>
      <h2 className="sr-only">{t('conversation.title')}</h2>
      <div className="conversation-panel__context">
        <UsersRound aria-hidden="true" />
        <span>
          {t(variant === 'direct' ? 'roomWorkspace.privateHint' : 'conversation.everyone')}
        </span>
        <span className="conversation-panel__room">{roomName}</span>
      </div>
      <div className="conversation-panel__history">
        <div
          className="conversation-panel__timeline"
          ref={timelineElement}
          onScroll={(event) => {
            const element = event.currentTarget;
            following.current =
              element.scrollHeight - element.scrollTop - element.clientHeight < 48;
            if (following.current) setUnseen(false);
          }}
          role="log"
          aria-label={t('conversation.title')}
          aria-live="polite"
          aria-relevant="additions text"
          aria-busy={state === 'loading'}
        >
          {state === 'loading' ? (
            <p className="conversation-panel__boundary">{t('conversation.loading')}</p>
          ) : state === 'failed' ? (
            <p className="conversation-panel__boundary" role="alert">
              {t('conversation.unavailable')}
            </p>
          ) : timeline.length === 0 ? (
            <motion.div
              className="conversation-panel__empty"
              initial={reduceMotion === true ? false : { opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ type: 'spring', stiffness: 300, damping: 30 }}
            >
              <div className="conversation-panel__empty-icon">
                <MessageCircle aria-hidden="true" />
              </div>
              <p className="conversation-panel__eyebrow">{t('conversation.emptyEyebrow')}</p>
              <h3>{t('conversation.emptyTitle')}</h3>
              <p>{t('conversation.empty')}</p>
              <div className="conversation-panel__starters">
                {(['introduce', 'idea'] as const).map((starter) => (
                  <button
                    type="button"
                    key={starter}
                    disabled={!canEdit}
                    onClick={() => {
                      composer.changeText(t(`conversation.starter.${starter}`));
                      input.current?.focus();
                    }}
                  >
                    {t(`conversation.starter.${starter}`)}
                    <ArrowUpRight aria-hidden="true" />
                  </button>
                ))}
              </div>
            </motion.div>
          ) : null}
          {timeline.map((message, index) => {
            const previous = timeline[index - 1];
            const day = new Date(message.serverTimestamp).toDateString();
            return (
              <div key={message.messageId}>
                {previous === undefined ||
                new Date(previous.serverTimestamp).toDateString() !== day ? (
                  <div className="conversation-day">
                    <span>{date.format(message.serverTimestamp)}</span>
                  </div>
                ) : null}
                <ConversationMessage
                  message={message}
                  parent={
                    message.relation === undefined
                      ? undefined
                      : messagesById.get(message.relation.targetMessageId)
                  }
                  names={names}
                  time={time}
                  editable={canEdit}
                  own={
                    message.actor.matrixUserId ===
                    composer.publication.context.identity?.matrixUserId
                  }
                  onReply={(target) => {
                    composer.respond(target);
                    input.current?.focus();
                  }}
                />
              </div>
            );
          })}
        </div>
        {unseen ? (
          <button
            className="conversation-panel__latest"
            type="button"
            onClick={() => {
              const element = timelineElement.current;
              if (element !== null) element.scrollTop = element.scrollHeight;
              following.current = true;
              setUnseen(false);
            }}
          >
            <ArrowDown aria-hidden="true" />
            {t('conversation.latest')}
          </button>
        ) : null}
      </div>
      <ConversationComposer
        composer={composer}
        participants={participants}
        names={names}
        roomId={roomId}
        canSend={canSend}
        writesAllowed={writesAllowed}
        input={input}
      />
      <div className="conversation-panel__footer">
        <details className="conversation-panel__availability">
          <summary>
            <Radio aria-hidden="true" />
            {t('conversation.details')}
          </summary>
          <div>
            <p>{t('conversation.runtime')}</p>
            <p>{t('conversation.help')}</p>
          </div>
        </details>
        <span className="conversation-panel__keyboard">{t('conversation.keyboard')}</span>
      </div>
    </section>
  );
}
