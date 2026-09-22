import { ArrowDown, Radio, UsersRound } from 'lucide-react';
import { motion, useReducedMotion } from 'motion/react';
import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from 'react';
import type { MessageRoomProjection } from '@/features/messages/domain/message';
import type { Result } from '@/shared/result';
import { emptyConversationFilter, searchConversation } from '../domain/conversation-search';
import { ConversationSearch } from './conversation-search';
import { useConversationPosition } from './use-conversation-position';
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
import { useReceptionEvidence } from '@/features/desktop/ui/reception-evidence-context';
import { conversationDeliveries } from '../domain/message-delivery';
import './conversation-panel.css';

const emptyParticipants: readonly ConversationParticipant[] = [];

export type ConversationPanelProps = {
  readonly active?: boolean;
  readonly history?: MessageRoomProjection['history'];
  readonly onLoadOlder?: () => Promise<
    Result<void, { readonly code: string; readonly retryable: boolean }>
  >;
  readonly focusMessageId?: string | null;
  readonly onLatestDisplayed?: (matrixEventId: string) => void;
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
  active = true,
  history,
  onLoadOlder,
  focusMessageId = null,
  onLatestDisplayed,
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
  const receivers = useReceptionEvidence();
  const reduceMotion = useReducedMotion();
  const input = useRef<HTMLTextAreaElement>(null);
  const composer = useConversationComposer(publisher, roomId, submissionIds);
  const timeline = useMemo(() => conversationMessages(messages), [messages]);
  const [filter, setFilter] = useState(emptyConversationFilter);
  const deferredFilter = useDeferredValue(filter);
  const visibleTimeline = useMemo(
    () => searchConversation(timeline, deferredFilter),
    [timeline, deferredFilter],
  );
  const filtered = Object.entries(filter).some(
    ([key, value]) => value !== (key === 'topic' ? null : ''),
  );
  const [historyState, setHistoryState] = useState<'ready' | 'loading' | 'failed'>('ready');
  const timelineElement = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const position = useConversationPosition({
    roomId,
    timeline,
    element: timelineElement,
    following,
    active,
    filtered,
    focusMessageId,
  });
  const markRead = position.markRead;
  const [unseen, setUnseen] = useState(false);
  const focusedMessage = useRef<HTMLDivElement>(null);
  const focusAvailable = visibleTimeline.some((message) => message.messageId === focusMessageId);
  const latestEvent = timeline.at(-1)?.matrixEventId;
  const displayedEvent = useRef<string | null>(null);
  const markLatestDisplayed = useCallback((): void => {
    const element = timelineElement.current;
    if (
      !active ||
      state !== 'ready' ||
      document.visibilityState === 'hidden' ||
      latestEvent === undefined ||
      filtered ||
      position.missing ||
      !following.current ||
      element === null ||
      element.clientHeight === 0 ||
      element.scrollHeight - element.scrollTop - element.clientHeight >= 48
    )
      return;
    if (displayedEvent.current !== latestEvent) {
      displayedEvent.current = latestEvent;
      onLatestDisplayed?.(latestEvent);
    }
    markRead();
  }, [
    active,
    latestEvent,
    onLatestDisplayed,
    state,
    filtered,
    position.missing,
    following,
    markRead,
  ]);
  useEffect(() => {
    const element = timelineElement.current;
    if (!active) return;
    if (following.current && element !== null && focusMessageId === null) {
      element.scrollTop = element.scrollHeight;
    } else if (latestEvent !== undefined) setUnseen(true);
  }, [active, latestEvent, focusMessageId]);
  useEffect(() => {
    markLatestDisplayed();
    document.addEventListener('visibilitychange', markLatestDisplayed);
    return () => {
      document.removeEventListener('visibilitychange', markLatestDisplayed);
    };
  }, [markLatestDisplayed]);
  useEffect(() => {
    if (!active || focusMessageId === null || focusedMessage.current === null) return;
    focusedMessage.current.scrollIntoView({ block: 'center' });
    focusedMessage.current.focus({ preventScroll: true });
  }, [active, focusMessageId, focusAvailable]);
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
  // 话题是回复关系连成的一串；只有参与了回复（回复过别人或被人回复）的消息才有话题可看，
  // 每条消息下都挂链接只是噪音。
  const topicMessageIds = useMemo(
    () =>
      new Set(
        messages.flatMap((message) =>
          message.relation === undefined
            ? []
            : [message.messageId, message.relation.targetMessageId],
        ),
      ),
    [messages],
  );
  const deliveries = useMemo(
    () => conversationDeliveries(timeline, receivers),
    [timeline, receivers],
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
      {/* 还没有任何消息时，搜索框和「已加载 0 条」只是噪音。 */}
      {timeline.length === 0 ? null : (
        <ConversationSearch
          messages={timeline}
          filter={filter}
          onChange={setFilter}
          count={visibleTimeline.length}
        />
      )}
      <div className="conversation-history-tools">
        {timeline.length === 0 ? null : (
          <span>{t('history.scope', { count: timeline.length })}</span>
        )}
        {history?.canLoadMore && onLoadOlder ? (
          <button
            type="button"
            disabled={historyState === 'loading'}
            onClick={() => {
              position.preserve();
              setHistoryState('loading');
              void onLoadOlder().then(
                (result) => {
                  setHistoryState(result.ok ? 'ready' : 'failed');
                },
                () => {
                  setHistoryState('failed');
                },
              );
            }}
          >
            {t(historyState === 'loading' ? 'history.loading' : 'history.older')}
          </button>
        ) : null}
        {history?.limited ? (
          <span>{t('history.limit')}</span>
        ) : history && !history.canLoadMore ? (
          <span>{t('history.complete')}</span>
        ) : null}
        {historyState === 'failed' ? <p role="alert">{t('history.failed')}</p> : null}
        {position.missing ? <p role="status">{t('history.positionMissing')}</p> : null}
        {position.missing || filtered ? (
          <button
            type="button"
            onClick={() => {
              setFilter(emptyConversationFilter);
              position.latest();
              requestAnimationFrame(() => {
                if (timelineElement.current)
                  timelineElement.current.scrollTop = timelineElement.current.scrollHeight;
              });
            }}
          >
            {t('history.latest')}
          </button>
        ) : null}
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
            markLatestDisplayed();
            position.record();
          }}
          role="log"
          // 面板矮时时间线会滚动；可滚动区域必须能用键盘聚焦，否则只用键盘的人翻不到旧消息。
          tabIndex={0}
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
              <h3>{t('conversation.emptyTitle')}</h3>
              <p>{t('conversation.empty')}</p>
            </motion.div>
          ) : null}
          {filtered && visibleTimeline.length === 0 ? (
            <p className="conversation-panel__boundary">{t('history.empty')}</p>
          ) : null}
          {visibleTimeline.map((message, index) => {
            const previous = visibleTimeline[index - 1];
            const day = new Date(message.serverTimestamp).toDateString();
            return (
              <div
                key={message.messageId}
                data-conversation-message-id={message.messageId}
                data-focused={message.messageId === focusMessageId}
                tabIndex={-1}
                ref={message.messageId === focusMessageId ? focusedMessage : undefined}
              >
                {previous === undefined ||
                new Date(previous.serverTimestamp).toDateString() !== day ? (
                  <div className="conversation-day">
                    <span>{date.format(message.serverTimestamp)}</span>
                  </div>
                ) : null}
                <ConversationMessage
                  delivery={deliveries.get(message.messageId) ?? []}
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
                {topicMessageIds.has(message.messageId) ? (
                  <button
                    type="button"
                    className="conversation-topic-link"
                    onClick={() => {
                      following.current = false;
                      setFilter({ ...emptyConversationFilter, topic: message.messageId });
                    }}
                  >
                    {t('history.viewTopic')}
                  </button>
                ) : null}
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
              position.latest();
              setUnseen(false);
              markLatestDisplayed();
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
