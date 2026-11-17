import {
  Check,
  CheckCheck,
  ChevronDown,
  ChevronUp,
  Circle,
  CircleCheck,
  Copy,
  MessageSquareShare,
  Pin,
  PinOff,
  Reply,
} from 'lucide-react';
import { useContext, useId, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { useTranslation } from 'react-i18next';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { ActorAvatar, actorTone } from '@/features/lobby/ui/avatars';
import { useIsNetworkAgent, useNetworkAgentLabel } from '@/features/lobby/ui/network-agent-labels';
import { RoomPinsActionsContext } from '@/features/room-pins/ui/room-pins-actions';
import { ExcerptSelectionContext } from '@/features/room-excerpts/ui/excerpt-selection';
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
  mentioned = false,
  onReply,
  onJump,
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
  /** 这一条在找你（点了你的名字，或者 @了所有人）：整行铺一层很浅的黄。 */
  readonly mentioned?: boolean;
  readonly onReply: (message: RoomMessageSignal) => void;
  /** 点“回复的是哪句”跳到那一句；不给，或者那一句没加载到、撤回了，就只是一行字。 */
  readonly onJump?: (messageId: string) => void;
  readonly time: Intl.DateTimeFormat;
  readonly delivery?: readonly AgentDelivery[];
  readonly expanded?: boolean;
  readonly onExpandedChange?: (expanded: boolean) => void;
}) {
  const { t } = useTranslation();
  const chat = message.preview?.conversation;
  const [copied, setCopied] = useState(false);
  // 房主和管理员在这间房的对话里能置顶、取消置顶一句。
  const pinActions = useContext(RoomPinsActionsContext);
  const pins =
    pinActions !== null && pinActions.roomId === message.roomId && message.lifecycle === 'active'
      ? pinActions
      : null;
  const pinned = pins?.pinned.has(message.matrixEventId) ?? false;
  // 放着实况的房间里，开了实况以后说的每一句都能“分享这一段”；正在选时每一句换成一个勾。
  const excerptActions = useContext(ExcerptSelectionContext);
  const excerpts =
    excerptActions !== null &&
    excerptActions.roomId === message.roomId &&
    excerptActions.shareable(message)
      ? excerptActions
      : null;
  const selecting = excerptActions?.roomId === message.roomId && excerptActions.selecting;
  const excerptPlace = excerpts?.place(message.messageId);
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
      data-mentioned={mentioned ? 'true' : undefined}
      data-action-count={selecting ? undefined : actionCount(chat?.text, pins, excerpts)}
      data-excerpt={excerptPlace}
      data-excerpt-pickable={selecting && excerpts !== null ? 'true' : undefined}
      onClick={
        selecting && excerpts !== null
          ? (event) => {
              // 选的时候点这一句的哪里都算；点里面的链接、按钮照旧是它们自己的。
              if (
                event.target instanceof Element &&
                event.target.closest('a, button, summary, input, textarea') !== null
              )
                return;
              excerpts.pick(message);
            }
          : undefined
      }
    >
      <div className="conversation-message__avatar" aria-hidden="true">
        <ActorAvatar actor={message.actor} />
      </div>
      <div className="conversation-message__body">
        <header>
          <strong className="ar-name" data-tone={actorTone(message.actor)}>
            {message.actor.displayName}
          </strong>
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
            {message.relation === undefined ? null : parent?.lifecycle === 'active' ? (
              <blockquote>
                {onJump === undefined ? (
                  `${parent.actor.displayName}: ${parent.preview?.summary ?? ''}`
                ) : (
                  <button
                    type="button"
                    className="conversation-message__quote"
                    title={t('conversation.jumpToQuoted')}
                    onClick={() => {
                      onJump(parent.messageId);
                    }}
                  >
                    {`${parent.actor.displayName}: ${parent.preview?.summary ?? ''}`}
                  </button>
                )}
              </blockquote>
            ) : (
              <blockquote>{t('conversation.referenced')}</blockquote>
            )}
            {chat?.mentions.length || message.preview?.mentionsEveryone === true ? (
              <div className="conversation-message__mentions">
                {message.preview?.mentionsEveryone === true ? (
                  <span>{t('conversation.everyoneChip')}</span>
                ) : null}
                {/* 认不出名字时不写整串 Matrix ID（它自己就带 @），写“某位成员”，ID 留在提示里。 */}
                {chat?.mentions.map((id) => (
                  <span key={id} title={id}>
                    @{names.get(id) ?? t('conversation.unknownMention')}
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
        {own ? <DeliveryMark delivery={delivery} /> : null}
      </div>
      {selecting ? (
        excerpts === null ? null : (
          <div className="conversation-message__actions conversation-message__actions--select">
            <button
              className="conversation-message__select"
              type="button"
              aria-pressed={excerptPlace !== undefined}
              aria-label={t('roomExcerpt.select.message', {
                name: message.actor.displayName,
                time: time.format(message.serverTimestamp),
              })}
              title={t('roomExcerpt.select.message', {
                name: message.actor.displayName,
                time: time.format(message.serverTimestamp),
              })}
              onClick={() => {
                excerpts.pick(message);
              }}
            >
              {excerptPlace === undefined ? (
                <Circle aria-hidden="true" />
              ) : (
                <CircleCheck aria-hidden="true" />
              )}
            </button>
          </div>
        )
      ) : (
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
          {pins === null ? null : (
            <button
              className="conversation-message__action"
              type="button"
              disabled={pins.pending.has(message.matrixEventId)}
              aria-label={t(pinned ? 'roomPins.action.unpin' : 'roomPins.action.pin')}
              title={t(pinned ? 'roomPins.action.unpin' : 'roomPins.action.pin')}
              onClick={() => {
                pins.toggle(message.matrixEventId, !pinned);
              }}
            >
              {pinned ? <PinOff aria-hidden="true" /> : <Pin aria-hidden="true" />}
            </button>
          )}
          {excerpts === null ? null : (
            <button
              className="conversation-message__action conversation-message__share"
              type="button"
              aria-label={t('roomExcerpt.action.share')}
              title={t('roomExcerpt.action.share')}
              onClick={() => {
                excerpts.begin(message);
              }}
            >
              <MessageSquareShare aria-hidden="true" />
            </button>
          )}
        </div>
      )}
    </article>
  );
}

/** 悬停出来的那一排有几个按钮：触屏上一直摆着，标题行右边按它留出位置，免得盖住时间。 */
function actionCount(text: string | undefined, pins: unknown, excerpts: unknown): number {
  return 1 + (text ? 1 : 0) + (pins === null ? 0 : 1) + (excerpts === null ? 0 : 1);
}

/**
 * 自己消息下的小标记：没有 Agent 回执时只说“已发送”；一个 Agent 时写它的进度；几个 Agent 时写个数。
 * 点开列出每个 Agent 的进度。有回复需要你处理时标记变成提醒色。
 */
function DeliveryMark({ delivery }: { readonly delivery: readonly AgentDelivery[] }) {
  const { t } = useTranslation();
  const [first] = delivery;
  const summary =
    first === undefined
      ? t('conversation.delivery.mark.sent')
      : delivery.length === 1
        ? `${first.name} · ${t(`conversation.delivery.${first.stage}`)}`
        : t('conversation.delivery.mark.agents', { count: delivery.length });
  const replied = first !== undefined && delivery.every((item) => item.stage === 'replied');
  return (
    <details
      className="conversation-message__delivery"
      data-attention={delivery.some((item) => item.stage === 'needs_review')}
    >
      <summary aria-label={`${t('conversation.delivery.title')}: ${summary}`}>
        {replied ? <CheckCheck aria-hidden="true" /> : <Check aria-hidden="true" />}
        <span>{summary}</span>
      </summary>
      <ul>
        <li>{t('conversation.delivery.sent')}</li>
        {first === undefined ? (
          <li>{t('conversation.delivery.unconfirmed')}</li>
        ) : (
          delivery.map((item) => (
            <li key={item.agentId} data-stage={item.stage}>
              {item.name} · {t(`conversation.delivery.${item.stage}`)}
            </li>
          ))
        )}
      </ul>
    </details>
  );
}
