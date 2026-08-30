import type { MessageContentReference, MessageActor } from '@/features/messages/domain/message';
import type { Result } from '@/shared/result';

export const handoffPermissions = ['read_text', 'read_attachments', 'include_metadata'] as const;
export const handoffPurposes = ['inspect', 'summarize', 'reply_draft'] as const;
export const handoffStatuses = [
  'approved',
  'delivered',
  'consumed',
  'declined',
  'revoked',
  'expired',
  'failed',
] as const;

export type HandoffPermission = (typeof handoffPermissions)[number];
export type HandoffPurpose = (typeof handoffPurposes)[number];
export type HandoffStatus = (typeof handoffStatuses)[number];

export type HandoffTarget = {
  readonly agentId: string;
  readonly displayName: string;
  readonly instanceId: string;
};

export type HandoffSource = {
  readonly actor: MessageActor;
  readonly content: MessageContentReference;
  readonly matrixEventId: string;
  readonly messageId: string;
  readonly riskFlags: readonly string[];
  readonly roomId: string;
};

export type HandoffApprovalRequest = {
  readonly expiresAtUnixMs: number;
  readonly handoffId: string;
  readonly permissions: readonly HandoffPermission[];
  readonly purpose: HandoffPurpose;
  readonly source: HandoffSource;
  readonly target: HandoffTarget;
};

export type HandoffSnapshot = {
  readonly expiresAtUnixMs: number;
  readonly failureCode?: string;
  readonly handoffId: string;
  readonly status: HandoffStatus;
};

export type HandoffSubmissionOutcome =
  | {
      readonly handoffId: string;
      readonly kind: 'submitted';
      readonly reused: boolean;
    }
  | {
      readonly handoffId: string;
      readonly kind: 'delivery_uncertain';
    }
  | {
      readonly kind: 'resolved';
      readonly snapshot: HandoffSnapshot;
    };

export type HandoffFailureCode =
  | 'handoff.bridge_unavailable'
  | 'handoff.targets_unavailable'
  | 'handoff.invalid_intent'
  | 'handoff.authorization_denied'
  | 'handoff.transport_rejected'
  | 'handoff.persistence_failed'
  | 'handoff.not_found'
  | 'handoff.already_resolved'
  | 'handoff.unexpected_failure';

export type HandoffFailure = {
  readonly code: HandoffFailureCode;
  readonly correlationId?: string;
  readonly retryable: boolean;
};

export type HandoffGateway = {
  approve(
    request: HandoffApprovalRequest,
  ): Promise<Result<HandoffSubmissionOutcome, HandoffFailure>>;
  listTargets(roomId: string): Promise<Result<readonly HandoffTarget[], HandoffFailure>>;
  reconcile(handoffId: string): Promise<Result<HandoffSnapshot, HandoffFailure>>;
  revoke(handoffId: string): Promise<Result<HandoffSnapshot, HandoffFailure>>;
};

export type HandoffRequestIssue =
  | 'content_scope_invalid'
  | 'expiry_invalid'
  | 'handoff_id_invalid'
  | 'source_invalid'
  | 'target_invalid';

const uuidV7Pattern = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;
const matrixRoomIdPattern = /^![^:]+:[^:]+$/u;
const matrixEventIdPattern = /^\$[^:]+(?::[^:]+)?$/u;
const maximumHandoffLifetimeMs = 60 * 60 * 1_000;

export function validateHandoffApproval(
  request: HandoffApprovalRequest,
  approvedAtUnixMs: number,
): readonly HandoffRequestIssue[] {
  const issues = new Set<HandoffRequestIssue>();
  if (!uuidV7Pattern.test(request.handoffId)) {
    issues.add('handoff_id_invalid');
  }
  if (!validTarget(request.target)) {
    issues.add('target_invalid');
  }
  if (!validSource(request.source)) {
    issues.add('source_invalid');
  }
  if (!validContentScope(request.permissions, request.source.content.mediaType)) {
    issues.add('content_scope_invalid');
  }
  const lifetime = request.expiresAtUnixMs - approvedAtUnixMs;
  if (
    !Number.isSafeInteger(request.expiresAtUnixMs) ||
    lifetime <= 0 ||
    lifetime > maximumHandoffLifetimeMs
  ) {
    issues.add('expiry_invalid');
  }
  return Object.freeze([...issues]);
}

export function isHandoffActive(status: HandoffStatus): boolean {
  return status === 'approved' || status === 'delivered';
}

function validTarget(target: HandoffTarget): boolean {
  return (
    uuidV7Pattern.test(target.agentId) &&
    uuidV7Pattern.test(target.instanceId) &&
    validDisplayName(target.displayName)
  );
}

function validDisplayName(value: string): boolean {
  let characterCount = 0;
  for (const character of value) {
    characterCount += 1;
    if (isControlCharacter(character)) {
      return false;
    }
  }
  return value.trim().length > 0 && characterCount <= 80;
}

function validSource(source: HandoffSource): boolean {
  return (
    matrixRoomIdPattern.test(source.roomId) &&
    matrixEventIdPattern.test(source.matrixEventId) &&
    uuidV7Pattern.test(source.messageId) &&
    source.actor.kind === 'agent' &&
    uuidV7Pattern.test(source.actor.agentId) &&
    uuidV7Pattern.test(source.actor.instanceId) &&
    uuidV7Pattern.test(source.content.contentId) &&
    /^[0-9a-f]{64}$/u.test(source.content.digestSha256) &&
    Number.isSafeInteger(source.content.sizeBytes) &&
    source.content.sizeBytes >= 0
  );
}

function validContentScope(permissions: readonly HandoffPermission[], mediaType: string): boolean {
  const unique = new Set(permissions);
  if (
    unique.size === 0 ||
    unique.size !== permissions.length ||
    [...unique].some((permission) => !handoffPermissions.includes(permission))
  ) {
    return false;
  }
  return mediaType.startsWith('text/')
    ? unique.has('read_text') && !unique.has('read_attachments')
    : unique.has('read_attachments') && !unique.has('read_text');
}

function isControlCharacter(character: string): boolean {
  const code = character.codePointAt(0) ?? 0;
  return code <= 31 || code === 127;
}
