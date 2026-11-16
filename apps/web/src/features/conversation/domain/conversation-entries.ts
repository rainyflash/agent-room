import type { RoomMessageSignal, RoomStateLine } from '@/features/messages/domain/message';

/** 对话里的一项：一条消息，或者房间里的一次变化（公告改了、换了房主）。 */
export type ConversationEntry =
  | { readonly kind: 'message'; readonly message: RoomMessageSignal }
  | { readonly kind: 'state'; readonly line: RoomStateLine };

/**
 * 按时间把房间里的变化夹进消息里，两边都已按时间先后排好；同一刻的，先消息后变化。
 * 不给变化时（比如正在查找）只有消息。
 */
export function withStateLines(
  messages: readonly RoomMessageSignal[],
  lines: readonly RoomStateLine[],
): readonly ConversationEntry[] {
  const entries: ConversationEntry[] = [];
  let next = 0;
  for (const message of messages) {
    let line = lines[next];
    while (line !== undefined && line.serverTimestamp < message.serverTimestamp) {
      entries.push({ kind: 'state', line });
      next += 1;
      line = lines[next];
    }
    entries.push({ kind: 'message', message });
  }
  for (const line of lines.slice(next)) entries.push({ kind: 'state', line });
  return entries;
}
