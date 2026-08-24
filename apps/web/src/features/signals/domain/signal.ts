import type { MessageLifecycle, MessageProvenance } from '@/features/messages/domain/message';

export const signalKinds = [
  'room_message',
  'direct_message',
  'mention',
  'task_reference',
  'handoff_pending',
  'sync_issue',
] as const;

export type SignalKind = (typeof signalKinds)[number];

export type SignalAction =
  | { readonly kind: 'open_message'; readonly messageId: string }
  | { readonly kind: 'open_task'; readonly taskId: string }
  | { readonly handoffId: string; readonly kind: 'review_handoff' }
  | { readonly kind: 'retry_sync' };

export type SignalActor = {
  readonly agentId: string;
  readonly avatarUrl?: string;
  readonly displayName: string;
  readonly instanceId: string;
  readonly matrixUserId: string;
  readonly provenance: MessageProvenance;
};

export type SignalProjection = {
  readonly action: SignalAction;
  readonly actor: SignalActor | null;
  readonly edited: boolean;
  readonly kind: SignalKind;
  readonly lifecycle: MessageLifecycle;
  readonly occurredAtUnixMs: number;
  readonly riskFlags: readonly string[];
  readonly signalId: string;
  readonly summary: string | null;
  readonly title: string | null;
};

const kindPriority: Readonly<Record<SignalKind, number>> = Object.freeze({
  direct_message: 30,
  handoff_pending: 50,
  mention: 40,
  room_message: 10,
  sync_issue: 60,
  task_reference: 20,
});

export function orderSignalProjections(
  signals: readonly SignalProjection[],
): readonly SignalProjection[] {
  return Object.freeze([...signals].toSorted(compareSignals));
}

function compareSignals(left: SignalProjection, right: SignalProjection): number {
  const urgencyDifference = urgency(right) - urgency(left);
  if (urgencyDifference !== 0) {
    return urgencyDifference;
  }
  const timestampDifference = right.occurredAtUnixMs - left.occurredAtUnixMs;
  return timestampDifference === 0
    ? right.signalId.localeCompare(left.signalId)
    : timestampDifference;
}

function urgency(signal: SignalProjection): number {
  return kindPriority[signal.kind] + Math.min(signal.riskFlags.length, 3) * 4;
}
