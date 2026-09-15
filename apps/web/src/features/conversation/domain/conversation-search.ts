import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { conversationMessages } from './conversation';

export type ConversationFilter = {
  readonly text: string;
  readonly actor: string;
  readonly from: string;
  readonly until: string;
  readonly topic: string | null;
};
export const emptyConversationFilter: ConversationFilter = Object.freeze({
  text: '',
  actor: '',
  from: '',
  until: '',
  topic: null,
});

export function conversationActorId(message: RoomMessageSignal): string {
  return message.actor.kind === 'agent'
    ? `agent:${message.actor.agentId}`
    : `human:${message.actor.principalId}`;
}

export function validSearchDates(filter: Pick<ConversationFilter, 'from' | 'until'>): boolean {
  return (
    [filter.from, filter.until].every(
      (value) =>
        value === '' ||
        (/^\d{4}-\d{2}-\d{2}$/u.test(value) &&
          Number.isFinite(Date.parse(value)) &&
          new Date(value).toISOString().slice(0, 10) === value),
    ) &&
    (filter.from === '' || filter.until === '' || filter.from <= filter.until)
  );
}

export function searchConversation(
  messages: readonly RoomMessageSignal[],
  filter: ConversationFilter,
): readonly RoomMessageSignal[] {
  if (!validSearchDates(filter)) return [];
  const timeline = conversationMessages(messages);
  const topic = filter.topic === null ? null : conversationTopic(timeline, filter.topic);
  const text = filter.text.trim().toLocaleLowerCase();
  return timeline.filter((message) => {
    const date = localDate(message.serverTimestamp);
    return (
      (topic === null || topic.has(message.messageId)) &&
      (filter.actor === '' || conversationActorId(message) === filter.actor) &&
      (filter.from === '' || date >= filter.from) &&
      (filter.until === '' || date <= filter.until) &&
      (text === '' ||
        message.preview?.conversation?.text.toLocaleLowerCase().includes(text) === true)
    );
  });
}

export function conversationTopic(
  messages: readonly RoomMessageSignal[],
  selected: string,
): ReadonlySet<string> {
  const edges = new Map<string, Set<string>>();
  for (const message of messages) {
    if (!message.relation) continue;
    const parent = message.relation.targetMessageId;
    for (const [left, right] of [
      [message.messageId, parent],
      [parent, message.messageId],
    ] as const) {
      const adjacent = edges.get(left) ?? new Set<string>();
      adjacent.add(right);
      edges.set(left, adjacent);
    }
  }
  const visited = new Set<string>();
  const pending = [selected];
  while (pending.length > 0) {
    const current = pending.pop();
    if (current === undefined || visited.has(current)) continue;
    visited.add(current);
    pending.push(...(edges.get(current) ?? []));
  }
  return visited;
}

function localDate(timestamp: number): string {
  const date = new Date(timestamp);
  return `${String(date.getFullYear())}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
}
