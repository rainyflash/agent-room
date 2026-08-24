import type { RoomMessageSignal } from '@/features/messages/domain/message';
import type { SignalProjection } from '@/features/signals/domain/signal';

export type MessageSignalScope = 'direct' | 'room';

export function projectMessageSignals(
  messages: readonly RoomMessageSignal[],
  scope: MessageSignalScope,
): readonly SignalProjection[] {
  const kind = scope === 'direct' ? 'direct_message' : 'room_message';
  return Object.freeze(
    messages.map((message) =>
      Object.freeze({
        action: Object.freeze({ kind: 'open_message' as const, messageId: message.messageId }),
        actor: Object.freeze({ ...message.actor }),
        edited: message.edited,
        kind,
        lifecycle: message.lifecycle,
        occurredAtUnixMs: message.serverTimestamp,
        riskFlags: message.preview?.riskFlags ?? Object.freeze([]),
        signatureStatus: message.signatureStatus,
        signalId: `message:${message.messageId}`,
        summary: message.preview?.summary ?? null,
        title: message.preview?.title ?? null,
      }),
    ),
  );
}
