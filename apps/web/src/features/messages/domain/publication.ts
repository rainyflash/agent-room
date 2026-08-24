import type { MessageProvenance, MessageSensitivity } from './message';
import type { Result } from '@/shared/result';

export const publicationMediaTypes = ['text/markdown', 'text/plain'] as const;
export const publicationProgressStages = ['uploading', 'submitting', 'binding'] as const;

export type PublicationMediaType = (typeof publicationMediaTypes)[number];
export type PublicationProgressStage = (typeof publicationProgressStages)[number];

export type MessagePublisherIdentity = {
  readonly agentId: string;
  readonly displayName: string;
  readonly instanceId: string;
  readonly matrixUserId: string;
  readonly provenance: Extract<MessageProvenance, 'human_confirmed_agent' | 'autonomous_agent'>;
  readonly source: 'bridge_agent_instance';
};

export type MessagePublicationDraft = {
  readonly body: string;
  readonly language?: string;
  readonly mediaType: PublicationMediaType;
  readonly riskFlags: readonly string[];
  readonly sensitivity: MessageSensitivity;
  readonly summary: string;
  readonly title: string;
};

export type MessagePublicationRequest = MessagePublicationDraft & {
  readonly roomId: string;
  readonly submissionId: string;
};

export type MessagePublicationOutcome =
  | {
      readonly kind: 'published';
      readonly matrixEventId: string;
      readonly reused: boolean;
      readonly submissionId: string;
    }
  | {
      readonly kind: 'accepted_binding_pending';
      readonly matrixEventId: string;
      readonly submissionId: string;
    }
  | {
      readonly kind: 'pending_reconciliation';
      readonly submissionId: string;
      readonly transactionId: string;
    };

export type MessagePublicationFailureCode =
  | 'publication.bridge_unavailable'
  | 'publication.identity_unavailable'
  | 'publication.invalid_intent'
  | 'publication.content_rejected'
  | 'publication.matrix_rejected'
  | 'publication.persistence_failed'
  | 'publication.unexpected_failure';

export type MessagePublicationFailure = {
  readonly code: MessagePublicationFailureCode;
  readonly correlationId?: string;
  readonly retryable: boolean;
};

export type MessagePublicationResult = Result<MessagePublicationOutcome, MessagePublicationFailure>;

export type MessagePublisher = {
  publish(
    request: MessagePublicationRequest,
    onProgress: (stage: PublicationProgressStage) => void,
  ): Promise<MessagePublicationResult>;
  reconcile(submissionId: string): Promise<MessagePublicationResult>;
  resolveIdentity(): Promise<Result<MessagePublisherIdentity, MessagePublicationFailure>>;
};

export type PublicationDraftIssue =
  | 'body_empty'
  | 'body_too_large'
  | 'language_invalid'
  | 'risk_flags_invalid'
  | 'summary_invalid'
  | 'title_invalid';

export type PublicationRequestIssue = PublicationDraftIssue | 'room_invalid' | 'submission_invalid';

const MAX_BODY_BYTES = 25 * 1_024 * 1_024;
const MAX_RISK_FLAGS = 16;
const MAX_RISK_FLAG_LENGTH = 64;
const MAX_SUMMARY_CHARACTERS = 500;
const MAX_TITLE_CHARACTERS = 120;
const languagePattern = /^[A-Za-z]{2,8}(?:-[A-Za-z0-9]{1,8})*$/u;
const riskFlagPattern = /^[a-z][a-z0-9_]*$/u;
const externalLinkPattern = /\b(?:https?:\/\/|www\.)\S+/iu;
const htmlMarkupPattern = /<\/?[A-Za-z][^>]{0,256}>/u;
const matrixRoomIdPattern = /^![^:]+:[^:]+$/u;
const uuidV7Pattern = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;

export function validatePublicationDraft(
  draft: MessagePublicationDraft,
): readonly PublicationDraftIssue[] {
  const issues = new Set<PublicationDraftIssue>();
  if (draft.body.trim().length === 0) {
    issues.add('body_empty');
  }
  if (new TextEncoder().encode(draft.body).byteLength > MAX_BODY_BYTES) {
    issues.add('body_too_large');
  }
  if (draft.language !== undefined && !languagePattern.test(draft.language)) {
    issues.add('language_invalid');
  }
  if (!validBoundedText(draft.title, MAX_TITLE_CHARACTERS)) {
    issues.add('title_invalid');
  }
  if (!validBoundedText(draft.summary, MAX_SUMMARY_CHARACTERS)) {
    issues.add('summary_invalid');
  }
  const uniqueRiskFlags = new Set(draft.riskFlags);
  if (
    uniqueRiskFlags.size !== draft.riskFlags.length ||
    uniqueRiskFlags.size > MAX_RISK_FLAGS ||
    [...uniqueRiskFlags].some(
      (flag) => flag.length > MAX_RISK_FLAG_LENGTH || !riskFlagPattern.test(flag),
    )
  ) {
    issues.add('risk_flags_invalid');
  }
  return Object.freeze([...issues]);
}

export function inspectPublicationRisks(body: string): readonly string[] {
  const flags = new Set<string>();
  if (externalLinkPattern.test(body)) {
    flags.add('external_links');
  }
  if (htmlMarkupPattern.test(body)) {
    flags.add('html_markup');
  }
  return Object.freeze([...flags]);
}

export function validatePublicationRequest(
  request: MessagePublicationRequest,
): readonly PublicationRequestIssue[] {
  const issues = new Set<PublicationRequestIssue>(validatePublicationDraft(request));
  if (!matrixRoomIdPattern.test(request.roomId) || request.roomId.length > 255) {
    issues.add('room_invalid');
  }
  if (!uuidV7Pattern.test(request.submissionId)) {
    issues.add('submission_invalid');
  }
  return Object.freeze([...issues]);
}

function validBoundedText(value: string, maximumCharacters: number): boolean {
  let characterCount = 0;
  let containsControlCharacter = false;
  for (const character of value) {
    characterCount += 1;
    const code = character.codePointAt(0) ?? 0;
    containsControlCharacter ||= code <= 31 || code === 127;
  }
  return value.length > 0 && characterCount <= maximumCharacters && !containsControlCharacter;
}
