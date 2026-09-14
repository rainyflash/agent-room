import type { RoomMessageSignal } from '@/features/messages/domain/message';

export type ConnectedInvitation = {
  readonly agentId: string;
  readonly displayName: string;
  readonly roomId: string;
  readonly startedAt: number;
};

export function inviteReplyProgress(
  messages: readonly RoomMessageSignal[],
  target: {
    readonly agentId: string;
    readonly roomId: string;
    readonly principalId: string;
    readonly startedAt: number;
  },
): 'message' | 'reply' | 'complete' {
  const requests = new Set(
    messages
      .filter(
        (message) =>
          message.roomId === target.roomId &&
          message.lifecycle === 'active' &&
          message.serverTimestamp >= target.startedAt &&
          message.actor.kind === 'human' &&
          message.actor.principalId === target.principalId,
      )
      .map((message) => message.messageId),
  );
  if (requests.size === 0) return 'message';
  return messages.some(
    (message) =>
      message.roomId === target.roomId &&
      message.lifecycle === 'active' &&
      message.actor.kind === 'agent' &&
      message.actor.agentId === target.agentId &&
      message.relation !== undefined &&
      requests.has(message.relation.targetMessageId),
  )
    ? 'complete'
    : 'reply';
}
