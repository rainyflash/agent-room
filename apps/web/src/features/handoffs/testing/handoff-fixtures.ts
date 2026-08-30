import type {
  HandoffApprovalRequest,
  HandoffSnapshot,
  HandoffStatus,
  HandoffTarget,
} from '@/features/handoffs/domain/handoff';

export const handoffTargetFixture: HandoffTarget = Object.freeze({
  adapterType: 'codex-desktop',
  agentAvatarContentId: null,
  agentDisplayName: 'Local Codex Agent',
  agentId: '01990d9e-8400-7000-8000-000000000011',
  capabilityVersion: '1',
  device: Object.freeze({
    deviceId: '01990d9e-8400-7000-8000-000000000013',
    label: 'Studio PC',
    platform: 'windows',
  }),
  instanceId: '01990d9e-8400-7000-8000-000000000012',
  instanceStatus: 'online',
  lastSeenAtUnixMs: 1_800_000_000_000,
  leaseExpiresAtUnixMs: 1_800_000_060_000,
  online: true,
});

export function handoffSnapshotFixture(
  handoffId: string,
  status: HandoffStatus = 'queued',
  expiresAtUnixMs = 1_800_000_000_000,
): HandoffSnapshot {
  const createdAtUnixMs = expiresAtUnixMs - 15 * 60_000;
  const delivered = status === 'delivered' || status === 'consumed';
  const terminal = ['consumed', 'declined', 'expired', 'failed', 'revoked'].includes(status);
  const resolvedAtUnixMs = terminal
    ? status === 'expired'
      ? expiresAtUnixMs
      : createdAtUnixMs + 2_000
    : null;
  return Object.freeze({
    consumedAtUnixMs: status === 'consumed' ? createdAtUnixMs + 2_000 : null,
    createdAtUnixMs,
    deliveredAtUnixMs: delivered ? createdAtUnixMs + 1_000 : null,
    expiresAtUnixMs,
    failureCode: ['declined', 'failed'].includes(status) ? 'handoff.fixture_failure' : null,
    handoffId,
    queuedAtUnixMs: createdAtUnixMs,
    resolvedAtUnixMs,
    status,
    targetAgentId: handoffTargetFixture.agentId,
    targetInstanceId: handoffTargetFixture.instanceId,
    version: status === 'queued' ? 0 : 1,
  });
}

export function acceptedHandoffFixture(request: HandoffApprovalRequest) {
  return Object.freeze({
    kind: 'accepted' as const,
    reused: false,
    snapshot: Object.freeze({
      ...handoffSnapshotFixture(request.handoffId, 'queued', request.expiresAtUnixMs),
      targetAgentId: request.target.agentId,
      targetInstanceId: request.target.instanceId,
    }),
  });
}
