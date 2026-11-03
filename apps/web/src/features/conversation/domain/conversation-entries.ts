import type { RoomMessageSignal, RoomNoticeChange } from '@/features/messages/domain/message';

/** 对话里的一项：一条消息，或者公告的一次改动。 */
export type ConversationEntry =
  | { readonly kind: 'message'; readonly message: RoomMessageSignal }
  | { readonly kind: 'notice'; readonly change: RoomNoticeChange };

/**
 * 按时间把公告的改动夹进消息里，两边都已按时间先后排好；同一刻的，先消息后改动。
 * 不给改动时（比如正在查找）只有消息。
 */
export function withNoticeChanges(
  messages: readonly RoomMessageSignal[],
  changes: readonly RoomNoticeChange[],
): readonly ConversationEntry[] {
  const entries: ConversationEntry[] = [];
  let next = 0;
  for (const message of messages) {
    let change = changes[next];
    while (change !== undefined && change.serverTimestamp < message.serverTimestamp) {
      entries.push({ kind: 'notice', change });
      next += 1;
      change = changes[next];
    }
    entries.push({ kind: 'message', message });
  }
  for (const change of changes.slice(next)) entries.push({ kind: 'notice', change });
  return entries;
}
