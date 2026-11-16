import type { RoomMessageSignal } from '@/features/messages/domain/message';

const eventFocusPrefix = 'matrix-event:';

/**
 * 按 Matrix 事件 ID 去看一句：置顶的那一句只知道事件 ID（`specs/redesign/brief-and-pins.md`“置顶”）。
 * 交给对话当要看的那一条；那一句已经加载了就换成它的消息 ID，还没加载到时对哪一条都对不上，对话照
 * “找不到就往前补”的那一套往前补，补到了再换过去。
 */
export function eventFocus(eventId: string): string {
  return `${eventFocusPrefix}${eventId}`;
}

/** 把 {@link eventFocus} 换成已经加载了的那一条的消息 ID；别的原样交回。 */
export function resolveFocus(
  focus: string | null,
  messages: readonly RoomMessageSignal[],
): string | null {
  if (!focus?.startsWith(eventFocusPrefix)) return focus;
  const eventId = focus.slice(eventFocusPrefix.length);
  return messages.find((message) => message.matrixEventId === eventId)?.messageId ?? focus;
}
