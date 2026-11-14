import type { RoomMessageSignal } from '@/features/messages/domain/message';
import {
  compareCursors,
  type ReadingCursor,
} from '@/features/personal-workspace/domain/workspace-document';

/** 一条消息在时间线上的位置，和“读到的位置”是同一种东西。 */
export function cursorOf(message: RoomMessageSignal): ReadingCursor {
  return { eventId: message.matrixEventId, timestamp: message.serverTimestamp };
}

export type NewMessages = {
  /** “新消息”那条线画在这一条前面；没有新的就是 null。 */
  readonly firstId: string | null;
  readonly count: number;
};
const none: NewMessages = { firstId: null, count: 0 };

/**
 * 上次看到的位置以后别人说的话：第一条是哪条、一共几条。你自己说的不算新消息。
 * `after` 是 null 时没有“上次”可比（这间房从没读到过底，或者你刚说过话），什么都不算新的。
 */
export function newMessagesAfter(
  timeline: readonly RoomMessageSignal[],
  self: string | null,
  after: ReadingCursor | null,
): NewMessages {
  if (after === null) return none;
  const fresh = timeline.filter(
    (message) =>
      message.actor.matrixUserId !== self && compareCursors(cursorOf(message), after) > 0,
  );
  return { firstId: fresh[0]?.messageId ?? null, count: fresh.length };
}
