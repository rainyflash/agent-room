import {
  addressesAccount,
  messageAttentionId,
  type RoomMessageSignal,
} from '@/features/messages/domain/message';
import {
  compareCursors,
  type ReadingCursor,
} from '@/features/personal-workspace/domain/workspace-document';

/** 对话上面那一行“只看”：全部、人说的、提到我的（`specs/redesign/room-page.md`）。 */
export const conversationScopes = ['all', 'humans', 'addressed'] as const;
export type ConversationScope = (typeof conversationScopes)[number];

/**
 * 照“只看”筛一遍，顺序不变。“提到我的”是别人点了你、@了所有人，或者回复你说的话；
 * 还不知道你是谁（`self` 是 null）时一条也没有。
 */
export function scopeConversation(
  timeline: readonly RoomMessageSignal[],
  scope: ConversationScope,
  self: string | null,
  byId: ReadonlyMap<string, RoomMessageSignal>,
): readonly RoomMessageSignal[] {
  if (scope === 'all') return timeline;
  if (scope === 'humans') return timeline.filter((message) => message.actor.kind === 'human');
  if (self === null) return [];
  return timeline.filter((message) => addressesAccount(message, byId, self) !== null);
}

/**
 * 读到的位置以后，有几条是冲着你来的、你还没看过。和收件箱是同一条规则：
 * 读到的位置以前的算看过，在收件箱里标过“看过了”的也算。
 * 还没有读到的位置（这间房从没读到过底）时不数，是 0。
 */
export function unreadAddressed(
  timeline: readonly RoomMessageSignal[],
  self: string | null,
  byId: ReadonlyMap<string, RoomMessageSignal>,
  read: {
    readonly cursor: ReadingCursor | undefined;
    readonly acknowledged: ReadonlySet<string>;
  },
): number {
  const { cursor, acknowledged } = read;
  if (self === null || cursor === undefined) return 0;
  return timeline.filter(
    (message) =>
      compareCursors(
        { eventId: message.matrixEventId, timestamp: message.serverTimestamp },
        cursor,
      ) > 0 &&
      addressesAccount(message, byId, self) !== null &&
      !acknowledged.has(messageAttentionId(message.roomId, message.messageId)),
  ).length;
}
