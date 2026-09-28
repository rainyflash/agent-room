import { Check, ChevronDown, ChevronUp, Copy, Reply } from 'lucide-react';
import { useId, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { useTranslation } from 'react-i18next';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { AgentPortrait } from '@/features/lobby/ui/room-illustration';
import { useIsNetworkAgent, useNetworkAgentLabel } from '@/features/lobby/ui/network-agent-labels';
import { initials } from '@/shared/ui/display-name';
import type { AgentDelivery } from '../domain/message-delivery';
import { ChatMarkdown } from './chat-markdown';
import { MessageAttachment } from './message-attachment';

// 收起时露出的高度约 11 行正文；比它高出不多的消息照常全文显示，免得“展开”只多出一两行。
const foldedHeight = 280;
const foldAbove = 400;

function useTallContent(element: RefObject<HTMLElement | null>): boolean {
  const [tall, setTall] = useState(false);
  useLayoutEffect(() => {
    const node = element.current;
    if (node === null) return;
    const measure = (): void => {
      setTall(node.scrollHeight > foldAbove);
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    // 面板宽度变化、图片加载完都会改变高度。收起时外框高度固定，内容再长也仍是长消息。
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    for (const child of node.children) observer.observe(child);
    return () => {
      observer.disconnect();
    };
  }, [element]);
  return tall;
}

export function ConversationMessage({
  message,
  parent,
  names,
  editable,
  own,
  onReply,
  time,
  delivery = [],
  expanded = false,
  onExpandedChange,
}: {
  readonly message: RoomMessageSignal;
  readonly parent: RoomMessageSignal | undefined;
  readonly names: ReadonlyMap<string, string>;
  readonly editable: boolean;
  readonly own: boolean;
  readonly onReply: (message: RoomMessageSignal) => void;
  readonly time: Intl.DateTimeFormat;
  readonly delivery?: readonly AgentDelivery[];
  readonly expanded?: boolean;
  readonly onExpandedChange?: (expanded: boolean) => void;
}) {
  const { t } = useTranslation();
  const chat = message.preview?.conversation;
  const [copied, setCopied] = useState(false);
  const fold = useRef<HTMLDivElement>(null);
  const foldId = useId();
  const tall = useTallContent(fold);
  const folded = tall && !expanded && onExpandedChange !== undefined;
  const network = useIsNetworkAgent(message.actor.kind === 'agent' ? message.actor.agentId : null);
  const networkLabel = useNetworkAgentLabel();
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
          {network ? (
            <span className="conversation-message__origin" title={networkLabel.hint}>
              {networkLabel.label}
            </span>
          ) : (
            <span>
              {t(message.actor.kind === 'human' ? 'conversation.human' : 'conversation.agent')}
            </span>
          )}
          <time dateTime={new Date(message.serverTimestamp).toISOString()}>
            {time.format(message.serverTimestamp)}
          </time>
        </header>
        <div className="conversation-message__text">
          <div
            className="conversation-message__fold"
            id={foldId}
            ref={fold}
            data-folded={folded}
            style={folded ? { maxHeight: `${String(foldedHeight)}px` } : undefined}
          >
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
              <ChatMarkdown source={chat?.text ?? ''} />
            )}
            {chat?.attachmentName !== undefined && message.content !== null ? (
              <MessageAttachment
                content={message.content}
                roomId={message.roomId}
                name={chat.attachmentName}
              />
            ) : null}
          </div>
          {tall && onExpandedChange !== undefined ? (
            <button
              type="button"
              className="conversation-message__fold-toggle"
              aria-expanded={!folded}
              aria-controls={foldId}
              onClick={(event) => {
                onExpandedChange(folded);
                // 收起很长的消息后，开头若已翻出视野就把它对齐到顶部，免得读到一半的人找不到位置。
                // 手机上收起后的消息可能仍比时间线高，nearest 在这种情况下不会滚动。
                if (!folded) {
                  const article = event.currentTarget.closest('article');
                  const timeline = article?.closest('[role="log"]');
                  requestAnimationFrame(() => {
                    if (
                      article &&
                      timeline &&
                      article.getBoundingClientRect().top < timeline.getBoundingClientRect().top
                    )
                      article.scrollIntoView({ block: 'start' });
                  });
                }
              }}
            >
              {folded ? <ChevronDown aria-hidden="true" /> : <ChevronUp aria-hidden="true" />}
              {t(folded ? 'conversation.expandMessage' : 'conversation.collapseMessage')}
            </button>
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
      <div className="conversation-message__actions">
        {chat?.text ? (
          <button
            className="conversation-message__action"
            type="button"
            aria-label={t(copied ? 'conversation.copied' : 'conversation.copy')}
            title={t(copied ? 'conversation.copied' : 'conversation.copy')}
            onClick={() => {
              void navigator.clipboard.writeText(chat.text).then(
                () => {
                  setCopied(true);
                  window.setTimeout(() => {
                    setCopied(false);
                  }, 1_500);
                },
                () => {
                  setCopied(false);
                },
              );
            }}
          >
            {copied ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}
          </button>
        ) : null}
        <button
          className="conversation-message__action"
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
      </div>
    </article>
  );
}
