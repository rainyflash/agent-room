import { Paperclip } from 'lucide-react';
import { useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';

import { ChatMarkdown } from '@/features/conversation/ui/chat-markdown';
import { AgentPortrait } from '@/features/lobby/ui/room-illustration';
import type {
  PublicWatch,
  PublicWatchMessage,
  PublicWatchParticipant,
} from '@/features/public-watch/domain/public-watch';
import { initials } from '@/shared/ui/display-name';
import '@/features/conversation/ui/conversation-panel.css';

/** 离底部不到这么多像素时算在看最新的：新消息来了跟着滚到底，往上翻着看时不打扰。 */
const followThresholdPx = 48;
/** 引用里最多露出被回复那条的这么多个字。 */
const quoteCharacters = 80;

/**
 * 围观页的对话：只能看。和房间里的对话用同一套样子，但没有输入框、回复和复制按钮，
 * 也不链到任何地方。
 */
/** 要滚到的那一条。每次点都给一个新对象：点同一个人两次，也会再滚过去。 */
export type WatchFocus = { readonly key: string };

export function PublicWatchMessages({
  watch,
  focus,
}: {
  readonly watch: PublicWatch;
  /** 点了场景里的气泡或人物：滚到那一条。 */
  readonly focus: WatchFocus | null;
}) {
  const { i18n, t } = useTranslation();
  const timeline = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const people = useMemo(
    () => new Map(watch.participants.map((participant) => [participant.key, participant])),
    [watch.participants],
  );
  const messages = useMemo(
    () => new Map(watch.messages.map((message) => [message.key, message])),
    [watch.messages],
  );
  const language = i18n.resolvedLanguage ?? i18n.language;
  const time = useMemo(
    () => new Intl.DateTimeFormat(language, { hour: '2-digit', minute: '2-digit' }),
    [language],
  );
  const date = useMemo(
    () => new Intl.DateTimeFormat(language, { month: 'long', day: 'numeric' }),
    [language],
  );
  const latestKey = watch.messages.at(-1)?.key ?? null;

  useLayoutEffect(() => {
    const element = timeline.current;
    if (element !== null && following.current) element.scrollTop = element.scrollHeight;
  }, [latestKey]);

  useEffect(() => {
    if (focus === null) return;
    const element = timeline.current?.querySelector<HTMLElement>(
      `[data-message-key="${CSS.escape(focus.key)}"]`,
    );
    element?.scrollIntoView({ block: 'center' });
    element?.focus({ preventScroll: true });
  }, [focus]);

  return (
    <div className="conversation-panel__history">
      <div
        className="conversation-panel__timeline"
        ref={timeline}
        onScroll={(event) => {
          const element = event.currentTarget;
          following.current =
            element.scrollHeight - element.scrollTop - element.clientHeight < followThresholdPx;
        }}
        role="log"
        // 面板矮时对话会滚动；可滚动区域要能用键盘聚焦，只用键盘的人才翻得到前面的。
        tabIndex={0}
        aria-label={t('roomGame.chat')}
        aria-live="polite"
        aria-relevant="additions text"
      >
        {watch.messages.length === 0 ? (
          <div className="conversation-panel__empty">
            <h3>{t('publicWatch.empty.title')}</h3>
            <p>{t('publicWatch.empty.detail')}</p>
          </div>
        ) : null}
        {watch.messages.map((message, index) => {
          const previous = watch.messages[index - 1];
          const day = new Date(message.sentAtUnixMs).toDateString();
          return (
            <div
              key={message.key}
              data-message-key={message.key}
              data-focused={message.key === focus?.key}
              tabIndex={-1}
            >
              {previous === undefined || new Date(previous.sentAtUnixMs).toDateString() !== day ? (
                <div className="conversation-day">
                  <span>{date.format(message.sentAtUnixMs)}</span>
                </div>
              ) : null}
              <WatchMessage
                author={people.get(message.author)}
                message={message}
                parent={message.replyTo === null ? undefined : messages.get(message.replyTo)}
                people={people}
                time={time}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}

function WatchMessage({
  author,
  message,
  parent,
  people,
  time,
}: {
  readonly author: PublicWatchParticipant | undefined;
  readonly message: PublicWatchMessage;
  readonly parent: PublicWatchMessage | undefined;
  readonly people: ReadonlyMap<string, PublicWatchParticipant>;
  readonly time: Intl.DateTimeFormat;
}) {
  const { t } = useTranslation();
  const name = author?.name ?? '';
  const agent = author !== undefined && author.kind !== 'person';
  return (
    <article className="conversation-message" data-actor-kind={agent ? 'agent' : 'human'}>
      <div className="conversation-message__avatar" aria-hidden="true">
        {agent ? <AgentPortrait id={message.author} /> : initials(name)}
      </div>
      <div className="conversation-message__body">
        <header>
          <strong>{name}</strong>
          {author?.kind === 'networkAgent' ? (
            <span className="conversation-message__origin">{t('lobby.agent.network')}</span>
          ) : (
            <span>{t(agent ? 'conversation.agent' : 'conversation.human')}</span>
          )}
          {message.edited ? (
            <span className="public-watch__edited">{t('messages.preview.edited')}</span>
          ) : null}
          <time dateTime={new Date(message.sentAtUnixMs).toISOString()}>
            {time.format(message.sentAtUnixMs)}
          </time>
        </header>
        <div className="conversation-message__text">
          {message.replyTo === null ? null : (
            <blockquote>
              {quoteText(
                quote(parent, people),
                t('conversation.referenced'),
                t('publicWatch.withheld'),
              )}
            </blockquote>
          )}
          {message.withheld ? (
            <p className="public-watch__withheld">{t('publicWatch.withheld')}</p>
          ) : message.text.trim() === '' ? null : (
            <ChatMarkdown source={message.text} />
          )}
          {message.truncated && !message.withheld ? (
            <p className="public-watch__note">{t('publicWatch.truncated')}</p>
          ) : null}
          {message.attachment && !message.withheld ? (
            <p className="public-watch__attachment">
              <Paperclip aria-hidden="true" />
              {t('publicWatch.attachment')}
            </p>
          ) : null}
        </div>
      </div>
    </article>
  );
}

type Quote = { readonly name: string; readonly excerpt: string | null };

/** 被回复的那条：在这一页里就给作者和开头（不公开的没有开头），不在这一页里是 null。 */
function quote(
  parent: PublicWatchMessage | undefined,
  people: ReadonlyMap<string, PublicWatchParticipant>,
): Quote | null {
  if (parent === undefined) return null;
  const name = people.get(parent.author)?.name ?? '';
  if (parent.withheld) return { excerpt: null, name };
  const characters = Array.from(parent.text.replace(/\s+/gu, ' ').trim());
  const excerpt =
    characters.length > quoteCharacters
      ? `${characters.slice(0, quoteCharacters - 1).join('')}…`
      : characters.join('');
  return { excerpt, name };
}

/** 不在这一页里的说“更早的消息”，不公开的说“一条不公开的消息”。 */
function quoteText(parent: Quote | null, earlier: string, withheld: string): string {
  if (parent === null) return earlier;
  return `${parent.name}: ${parent.excerpt ?? withheld}`;
}
