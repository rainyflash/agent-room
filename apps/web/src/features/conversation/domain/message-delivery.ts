import type { RoomMessageSignal } from '@/features/messages/domain/message';
import type { ReceiverView } from '@/features/desktop/domain/reception';

export type AgentDelivery = {
  readonly agentId: string;
  readonly name: string;
  readonly stage: 'received' | 'running' | 'verifying' | 'replied' | 'needs_review' | 'skipped';
};

export function conversationDeliveries(
  messages: readonly RoomMessageSignal[],
  receivers: readonly ReceiverView[],
) {
  const replies = new Map<string, RoomMessageSignal[]>();
  const deliveries = new Map<string, ReceiverView[]>();
  for (const message of messages) {
    if (message.relation === undefined) continue;
    const key = message.relation.targetMessageId;
    const group = replies.get(key) ?? [];
    group.push(message);
    replies.set(key, group);
  }
  for (const view of receivers) {
    const record =
      view.progress?.type === 'delivery' ? view.progress.record : view.state.lastDelivery;
    if (record === null) continue;
    const group = deliveries.get(record.eventId) ?? [];
    group.push(view);
    deliveries.set(record.eventId, group);
  }
  return new Map(
    messages.map((message) => [
      message.messageId,
      messageDelivery(
        message,
        replies.get(message.messageId) ?? [],
        deliveries.get(message.matrixEventId) ?? [],
      ),
    ]),
  );
}

/** Presence and poll timestamps cannot acknowledge a particular message. */
export function messageDelivery(
  message: RoomMessageSignal,
  replies: readonly RoomMessageSignal[],
  receivers: readonly ReceiverView[],
): readonly AgentDelivery[] {
  const evidence = new Map<string, AgentDelivery>();
  for (const view of receivers) {
    const agentId = view.state.agentId;
    if (agentId === null || view.state.binding.policy.roomId !== message.roomId) continue;
    const record =
      view.progress?.type === 'delivery' ? view.progress.record : view.state.lastDelivery;
    if (record?.eventId !== message.matrixEventId || record.messageId !== message.messageId)
      continue;
    evidence.set(agentId, {
      agentId,
      name: view.state.binding.session.displayName,
      stage:
        record.failure || (record.stage === 'running' && !view.running)
          ? 'needs_review'
          : record.stage,
    });
  }
  for (const reply of replies) {
    if (
      reply.roomId !== message.roomId ||
      reply.actor.kind !== 'agent' ||
      reply.lifecycle !== 'active' ||
      reply.relation?.targetMessageId !== message.messageId
    )
      continue;
    evidence.set(reply.actor.agentId, {
      agentId: reply.actor.agentId,
      name: reply.actor.displayName,
      stage: 'replied',
    });
  }
  return [...evidence.values()];
}
