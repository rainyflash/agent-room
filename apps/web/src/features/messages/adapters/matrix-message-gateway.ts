import { contentEncryptionSchema } from './content-encryption-schema';
import { conversationSchema } from '@/features/conversation/adapters/conversation-schema';
import { z } from 'zod';

import {
  matrixMessagePreviewEventType,
  matrixMessagePreviewEventTypeV2,
  matrixMessageRevisionEventType,
  matrixMessageRevisionEventTypeV2,
  matrixModerationNoticeEventType,
  matrixAgentRoomEventNamespace,
  type MatrixMessageRoomSnapshot,
  type MatrixMessageSource,
  type MatrixMessageTimelineEvent,
} from './matrix-message-source';
import {
  messageProvenances,
  messageSensitivities,
  type MessageActor,
  type MessageContentReference,
  type MessageGateway,
  type MessagePreview,
  type MessageReadResult,
  type MessageRelation,
  type MessageRoomProjection,
  type ReadOnlyFederatedEvent,
  type MessageSignatureStatus,
  type RoomMessageSignal,
} from '@/features/messages/domain/message';
import { err, ok } from '@/shared/result';

const MAX_PROJECTED_MESSAGES = 200;
const MAX_PROJECTED_READ_ONLY_EVENTS = 50;
const legacyMatrixAgentRoomEventNamespace = ['org', 'agentroom'].join('.');
const uuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u);
const matrixRoomIdSchema = z
  .string()
  .min(4)
  .max(255)
  .regex(/^![^:]+:[^:]+$/u);
const matrixUserIdSchema = z
  .string()
  .min(4)
  .max(255)
  .regex(/^@[^:]+:[^:]+$/u);
const mediaTypeSchema = z
  .string()
  .min(3)
  .max(128)
  .regex(/^[a-z0-9.+-]+\/[a-z0-9.+-]+$/u);
const legacyActorSchema = z
  .looseObject({
    agent: z
      .looseObject({
        agentId: uuidV7Schema,
        avatarUrl: z
          .string()
          .max(2_048)
          .regex(/^https:\/\//u)
          .optional(),
        displayName: z
          .string()
          .min(1)
          .max(160)
          .refine((value) => Array.from(value).length <= 80),
        matrixUserId: matrixUserIdSchema,
      })
      .superRefine(limitProperties(16)),
    instanceId: uuidV7Schema,
    provenance: z.enum(messageProvenances),
  })
  .superRefine(limitProperties(12));
const agentReferenceSchema = z
  .looseObject({
    agentId: uuidV7Schema,
    avatarUrl: z
      .string()
      .max(2_048)
      .regex(/^https:\/\//u)
      .optional(),
    displayName: z
      .string()
      .min(1)
      .max(160)
      .refine((value) => Array.from(value).length <= 80),
    matrixUserId: matrixUserIdSchema,
  })
  .superRefine(limitProperties(16));
const humanActorSchema = z
  .looseObject({
    avatarUrl: z
      .string()
      .max(2_048)
      .regex(/^https:\/\//u)
      .optional(),
    displayName: z
      .string()
      .min(1)
      .max(160)
      .refine((value) => Array.from(value).length <= 80),
    kind: z.literal('human'),
    matrixUserId: matrixUserIdSchema,
    principalId: uuidV7Schema,
  })
  .superRefine(limitProperties(16));
const agentActorSchema = z
  .looseObject({
    agent: agentReferenceSchema,
    instanceId: uuidV7Schema,
    kind: z.literal('agent'),
    provenance: z.enum(['human_confirmed_agent', 'autonomous_agent']),
  })
  .superRefine(limitProperties(12));
const currentActorSchema = z.discriminatedUnion('kind', [humanActorSchema, agentActorSchema]);
const contentSchema = z
  .looseObject({
    encryption: contentEncryptionSchema.optional(),
    contentId: uuidV7Schema,
    digestSha256: z.string().regex(/^[0-9a-f]{64}$/u),
    fetchMode: z.literal('on_demand'),
    mediaType: mediaTypeSchema,
    sizeBytes: z
      .number()
      .int()
      .min(1)
      .max(25 * 1_024 * 1_024),
  })
  .superRefine(limitProperties(16));
const previewSchema = z
  .looseObject({
    conversation: conversationSchema.optional(),
    contentType: mediaTypeSchema,
    language: z
      .string()
      .min(2)
      .max(35)
      .regex(/^[A-Za-z]{2,8}(?:-[A-Za-z0-9]{1,8})*$/u)
      .optional(),
    riskFlags: z
      .array(
        z
          .string()
          .min(1)
          .max(64)
          .regex(/^[a-z][a-z0-9_]*$/u),
      )
      .max(16)
      .refine((flags) => new Set(flags).size === flags.length),
    sensitivity: z.enum(messageSensitivities),
    summary: z
      .string()
      .min(1)
      .max(1000)
      .refine((value) => Array.from(value).length <= 500),
    title: z
      .string()
      .min(1)
      .max(240)
      .refine((value) => Array.from(value).length <= 120),
  })
  .superRefine(limitProperties(16));
const relationSchema = z
  .looseObject({
    kind: z.literal('reply'),
    targetMessageId: uuidV7Schema,
  })
  .superRefine(limitProperties(8));
const signatureSchema = z
  .string()
  .min(43)
  .max(128)
  .regex(/^[A-Za-z0-9_-]+$/u);
const legacyCommonEventShape = {
  actor: legacyActorSchema,
  correlationId: z.uuid(),
  createdAt: z.iso.datetime({ offset: true }),
  id: uuidV7Schema,
  roomId: matrixRoomIdSchema,
  schemaVersion: z.literal('1.0'),
  signature: signatureSchema,
};
const currentCommonEventShape = {
  actor: currentActorSchema,
  correlationId: z.uuid(),
  createdAt: z.iso.datetime({ offset: true }),
  id: uuidV7Schema,
  roomId: matrixRoomIdSchema,
  schemaVersion: z.literal('2.0'),
  signature: signatureSchema.optional(),
};
const legacyPreviewEventSchema = z
  .looseObject({
    ...legacyCommonEventShape,
    content: contentSchema,
    eventType: z.literal(matrixMessagePreviewEventType),
    preview: previewSchema,
    relation: relationSchema.optional(),
  })
  .superRefine(limitProperties(24));
const currentPreviewEventSchema = z
  .looseObject({
    ...currentCommonEventShape,
    content: contentSchema,
    eventType: z.literal(matrixMessagePreviewEventTypeV2),
    preview: previewSchema,
    relation: relationSchema.optional(),
  })
  .superRefine((event, context) => {
    limitProperties(24)(event, context);
    const signatureMatchesActor =
      event.actor.kind === 'human' ? event.signature === undefined : event.signature !== undefined;
    if (!signatureMatchesActor) {
      context.addIssue({ code: 'custom', message: '签名存在性与主体类型不一致。' });
    }
  });
const previewEventSchema = z.union([legacyPreviewEventSchema, currentPreviewEventSchema]);
const legacyRevisionEventSchema = z
  .looseObject({
    ...legacyCommonEventShape,
    content: contentSchema.optional(),
    eventType: z.literal(matrixMessageRevisionEventType),
    kind: z.enum(['replace', 'redact', 'moderate']),
    preview: previewSchema.optional(),
    targetMessageId: uuidV7Schema,
  })
  .superRefine((revision, context) => {
    limitProperties(24)(revision, context);
    const hasReplacement = revision.preview !== undefined && revision.content !== undefined;
    if (revision.kind === 'replace' ? !hasReplacement : hasReplacement) {
      context.addIssue({ code: 'custom', message: '修订载荷与修订类型不一致。' });
    }
    if (
      revision.kind !== 'replace' &&
      (revision.preview !== undefined || revision.content !== undefined)
    ) {
      context.addIssue({ code: 'custom', message: '非替换修订不得携带正文引用。' });
    }
  });
const currentRevisionEventSchema = z
  .looseObject({
    ...currentCommonEventShape,
    content: contentSchema.optional(),
    eventType: z.literal(matrixMessageRevisionEventTypeV2),
    kind: z.enum(['replace', 'redact', 'moderate']),
    preview: previewSchema.optional(),
    targetMessageId: uuidV7Schema,
  })
  .superRefine((revision, context) => {
    limitProperties(24)(revision, context);
    const signatureMatchesActor =
      revision.actor.kind === 'human'
        ? revision.signature === undefined
        : revision.signature !== undefined;
    if (!signatureMatchesActor) {
      context.addIssue({ code: 'custom', message: '签名存在性与主体类型不一致。' });
    }
    const hasReplacement = revision.preview !== undefined && revision.content !== undefined;
    if (revision.kind === 'replace' ? !hasReplacement : hasReplacement) {
      context.addIssue({ code: 'custom', message: '修订载荷与修订类型不一致。' });
    }
    if (
      revision.kind !== 'replace' &&
      (revision.preview !== undefined || revision.content !== undefined)
    ) {
      context.addIssue({ code: 'custom', message: '非替换修订不得携带正文引用。' });
    }
  });
const revisionEventSchema = z.union([legacyRevisionEventSchema, currentRevisionEventSchema]);
const moderationNoticeSchema = z
  .object({
    actionId: uuidV7Schema,
    eventType: z.literal(matrixModerationNoticeEventType),
    hidden: z.boolean(),
    reasonCode: z.enum([
      'spam',
      'harassment',
      'impersonation',
      'malicious_content',
      'privacy_violation',
      'unsafe_automation',
      'other',
    ]),
    schemaVersion: z.literal('1.0'),
    targetEventId: z
      .string()
      .min(2)
      .max(1_024)
      .startsWith('$')
      .refine(hasNoControlCharacters, '事件标识不得包含控制字符。'),
  })
  .strict();

type ParsedPreviewEvent = z.output<typeof previewEventSchema>;
type ParsedRevisionEvent = z.output<typeof revisionEventSchema>;

type MutableMessage = {
  actor: MessageActor;
  content: MessageContentReference | null;
  edited: boolean;
  endToEndEncrypted: boolean;
  lifecycle: RoomMessageSignal['lifecycle'];
  matrixEventId: string;
  messageId: string;
  preview: MessagePreview | null;
  relation?: MessageRelation;
  roomId: string;
  serverTimestamp: number;
  signatureStatus: MessageSignatureStatus;
};

function hasNoControlCharacters(value: string): boolean {
  return Array.from(value).every((character) => {
    const codePoint = character.codePointAt(0);
    return codePoint !== undefined && codePoint >= 0x20 && codePoint !== 0x7f;
  });
}

type ParsedRevision = {
  readonly content?: MessageContentReference;
  readonly kind: ParsedRevisionEvent['kind'];
  readonly preview?: MessagePreview;
  readonly sender: string;
  readonly targetMessageId: string;
};

type ParsedModerationNotice = z.output<typeof moderationNoticeSchema> & {
  readonly serverTimestamp: number;
};

export class MatrixMessageGateway implements MessageGateway {
  readonly #now: () => number;
  readonly #source: MatrixMessageSource;

  constructor(source: MatrixMessageSource, now: () => number = Date.now) {
    this.#source = source;
    this.#now = now;
  }

  read(roomId: string): MessageReadResult {
    try {
      const read = this.#source.read(roomId);
      if (read.kind === 'matrix-unavailable') {
        return err({ code: 'messages.matrix_unavailable', retryable: true });
      }
      if (read.kind === 'room-not-joined') {
        return err({ code: 'messages.room_not_joined', retryable: true });
      }
      return ok(projectRoom(read.room, this.#now()));
    } catch {
      return err({ code: 'messages.projection_invalid', retryable: true });
    }
  }

  subscribe(roomId: string, listener: () => void): () => void {
    return this.#source.subscribe(roomId, listener);
  }
}

function projectRoom(
  room: MatrixMessageRoomSnapshot,
  observedAtUnixMs: number,
): MessageRoomProjection {
  const messages = new Map<string, MutableMessage>();
  const pendingRevisions = new Map<string, ParsedRevision[]>();
  const moderationNotices = new Map<string, ParsedModerationNotice>();
  const readOnlyFederatedEvents: ReadOnlyFederatedEvent[] = [];
  const seenMatrixEventIds = new Set<string>();

  for (const timelineEvent of room.timelineEvents) {
    const eventId = timelineEvent.eventId;
    if (eventId === undefined || seenMatrixEventIds.has(eventId)) {
      continue;
    }
    seenMatrixEventIds.add(eventId);
    if (
      timelineEvent.type === matrixMessagePreviewEventType ||
      timelineEvent.type === matrixMessagePreviewEventTypeV2
    ) {
      const parsed = parsePreview(room.roomId, timelineEvent);
      if (parsed === null || messages.has(parsed.messageId)) {
        continue;
      }
      messages.set(parsed.messageId, parsed);
      for (const revision of pendingRevisions.get(parsed.messageId) ?? []) {
        applyRevision(parsed, revision);
      }
      pendingRevisions.delete(parsed.messageId);
      continue;
    }
    if (
      timelineEvent.type === matrixMessageRevisionEventType ||
      timelineEvent.type === matrixMessageRevisionEventTypeV2
    ) {
      const revision = parseRevision(room.roomId, timelineEvent);
      if (revision === null) {
        continue;
      }
      const target = messages.get(revision.targetMessageId);
      if (target === undefined) {
        const pending = pendingRevisions.get(revision.targetMessageId) ?? [];
        pending.push(revision);
        pendingRevisions.set(revision.targetMessageId, pending);
      } else {
        applyRevision(target, revision);
      }
      continue;
    }
    if (timelineEvent.type === matrixModerationNoticeEventType) {
      const notice = parseModerationNotice(timelineEvent);
      if (notice !== null) {
        const current = moderationNotices.get(notice.targetEventId);
        if (current === undefined || current.serverTimestamp <= notice.serverTimestamp) {
          moderationNotices.set(notice.targetEventId, notice);
        }
      }
      continue;
    }
    const readOnlyEvent = projectReadOnlyFederatedEvent(timelineEvent);
    if (readOnlyEvent !== null) {
      readOnlyFederatedEvents.push(readOnlyEvent);
    }
  }

  for (const message of messages.values()) {
    const notice = moderationNotices.get(message.matrixEventId);
    if (notice?.hidden === true && message.lifecycle === 'active') {
      message.preview = null;
      message.content = null;
      message.lifecycle = 'moderated';
    }
  }

  const projected = [...messages.values()]
    .toSorted(compareMessages)
    .slice(0, MAX_PROJECTED_MESSAGES)
    .map(freezeMessage);
  return Object.freeze({
    messages: Object.freeze(projected),
    observedAtUnixMs,
    readOnlyFederatedEvents: Object.freeze(
      readOnlyFederatedEvents
        .toSorted(compareReadOnlyEvents)
        .slice(0, MAX_PROJECTED_READ_ONLY_EVENTS),
    ),
    roomId: room.roomId,
  });
}

function projectReadOnlyFederatedEvent(
  event: MatrixMessageTimelineEvent,
): ReadOnlyFederatedEvent | null {
  if (
    event.eventId === undefined ||
    event.sender === undefined ||
    !validServerTimestamp(event.serverTimestamp)
  ) {
    return null;
  }
  const reason = event.type.startsWith(`${legacyMatrixAgentRoomEventNamespace}.`)
    ? 'legacy_namespace'
    : event.type.startsWith(`${matrixAgentRoomEventNamespace}.`)
      ? 'unknown_event_type'
      : null;
  if (reason === null) {
    return null;
  }
  return Object.freeze({
    endToEndEncrypted: event.endToEndEncrypted,
    eventType: event.type,
    matrixEventId: event.eventId,
    reason,
    sender: event.sender,
    serverTimestamp: event.serverTimestamp,
  });
}

function parseModerationNotice(event: MatrixMessageTimelineEvent): ParsedModerationNotice | null {
  const parsed = moderationNoticeSchema.safeParse(event.content);
  if (!parsed.success || !validServerTimestamp(event.serverTimestamp)) {
    return null;
  }
  return { ...parsed.data, serverTimestamp: event.serverTimestamp };
}

function parsePreview(roomId: string, event: MatrixMessageTimelineEvent): MutableMessage | null {
  const parsed = previewEventSchema.safeParse(event.content);
  if (
    !parsed.success ||
    parsed.data.roomId !== roomId ||
    actorMatrixUserId(parsed.data) !== event.sender ||
    (parsed.data.content.encryption !== undefined &&
      (!event.endToEndEncrypted ||
        parsed.data.content.encryption.contextId !== parsed.data.id ||
        parsed.data.content.sizeBytes !==
          parsed.data.content.encryption.plaintextSizeBytes + 16)) ||
    parsed.data.preview.contentType !== parsed.data.content.mediaType ||
    (parsed.data.preview.conversation !== undefined &&
      parsed.data.preview.contentType !== 'text/plain') ||
    event.eventId === undefined ||
    !validServerTimestamp(event.serverTimestamp)
  ) {
    return null;
  }
  return {
    actor: toActor(parsed.data),
    content: toContent(parsed.data.content),
    edited: false,
    endToEndEncrypted: event.endToEndEncrypted,
    lifecycle: 'active',
    matrixEventId: event.eventId,
    messageId: parsed.data.id,
    preview: toPreview(parsed.data.preview),
    ...(parsed.data.relation === undefined ? {} : { relation: parsed.data.relation }),
    roomId,
    serverTimestamp: event.serverTimestamp,
    signatureStatus: 'matrix_sender_matched',
  };
}

function parseRevision(roomId: string, event: MatrixMessageTimelineEvent): ParsedRevision | null {
  const parsed = revisionEventSchema.safeParse(event.content);
  if (
    !parsed.success ||
    parsed.data.roomId !== roomId ||
    actorMatrixUserId(parsed.data) !== event.sender ||
    (parsed.data.content?.encryption !== undefined &&
      (!event.endToEndEncrypted ||
        parsed.data.content.encryption.contextId !== parsed.data.id ||
        parsed.data.content.sizeBytes !== parsed.data.content.encryption.plaintextSizeBytes + 16))
  ) {
    return null;
  }
  if (
    parsed.data.kind === 'replace' &&
    parsed.data.preview?.contentType !== parsed.data.content?.mediaType
  ) {
    return null;
  }
  return {
    ...(parsed.data.content === undefined ? {} : { content: toContent(parsed.data.content) }),
    kind: parsed.data.kind,
    ...(parsed.data.preview === undefined ? {} : { preview: toPreview(parsed.data.preview) }),
    sender: actorMatrixUserId(parsed.data),
    targetMessageId: parsed.data.targetMessageId,
  };
}

function applyRevision(target: MutableMessage, revision: ParsedRevision): void {
  if (target.actor.matrixUserId !== revision.sender || target.lifecycle !== 'active') {
    return;
  }
  if (revision.kind === 'replace') {
    if (revision.preview === undefined || revision.content === undefined) {
      return;
    }
    target.preview = revision.preview;
    target.content = revision.content;
    target.edited = true;
    return;
  }
  target.preview = null;
  target.content = null;
  target.lifecycle = revision.kind === 'redact' ? 'redacted' : 'moderated';
}

function toActor(event: ParsedPreviewEvent): MessageActor {
  if (event.schemaVersion === '2.0') {
    if (event.actor.kind === 'human') {
      return Object.freeze({
        ...(event.actor.avatarUrl === undefined ? {} : { avatarUrl: event.actor.avatarUrl }),
        displayName: event.actor.displayName,
        kind: 'human',
        matrixUserId: event.actor.matrixUserId,
        principalId: event.actor.principalId,
        provenance: 'human',
      });
    }
    const agent = event.actor.agent;
    return Object.freeze({
      agentId: agent.agentId,
      ...(agent.avatarUrl === undefined ? {} : { avatarUrl: agent.avatarUrl }),
      displayName: agent.displayName,
      instanceId: event.actor.instanceId,
      kind: 'agent',
      matrixUserId: agent.matrixUserId,
      provenance: event.actor.provenance,
    });
  }
  const agent = event.actor.agent;
  return Object.freeze({
    agentId: agent.agentId,
    ...(agent.avatarUrl === undefined ? {} : { avatarUrl: agent.avatarUrl }),
    displayName: agent.displayName,
    instanceId: event.actor.instanceId,
    kind: 'agent',
    matrixUserId: agent.matrixUserId,
    provenance:
      event.actor.provenance === 'autonomous_agent' ? 'autonomous_agent' : 'human_confirmed_agent',
  });
}

function actorMatrixUserId(event: ParsedPreviewEvent | ParsedRevisionEvent): string {
  if (event.schemaVersion === '1.0') {
    return event.actor.agent.matrixUserId;
  }
  return event.actor.kind === 'human' ? event.actor.matrixUserId : event.actor.agent.matrixUserId;
}

function toContent(content: z.output<typeof contentSchema>): MessageContentReference {
  return Object.freeze({
    ...(content.encryption === undefined ? {} : { encryption: content.encryption }),
    contentId: content.contentId,
    digestSha256: content.digestSha256,
    mediaType: content.mediaType,
    sizeBytes: content.sizeBytes,
  });
}

function toPreview(preview: z.output<typeof previewSchema>): MessagePreview {
  return Object.freeze({
    ...(preview.conversation === undefined
      ? {}
      : {
          conversation: Object.freeze({
            text: preview.conversation.text,
            mentions: Object.freeze([...preview.conversation.mentions]),
          }),
        }),
    contentType: preview.contentType,
    ...(preview.language === undefined ? {} : { language: preview.language }),
    riskFlags: Object.freeze([...preview.riskFlags]),
    sensitivity: preview.sensitivity,
    summary: preview.summary,
    title: preview.title,
  });
}

function freezeMessage(message: MutableMessage): RoomMessageSignal {
  return Object.freeze({
    actor: message.actor,
    content: message.content,
    edited: message.edited,
    endToEndEncrypted: message.endToEndEncrypted,
    lifecycle: message.lifecycle,
    matrixEventId: message.matrixEventId,
    messageId: message.messageId,
    preview: message.preview,
    ...(message.relation === undefined ? {} : { relation: message.relation }),
    roomId: message.roomId,
    serverTimestamp: message.serverTimestamp,
    signatureStatus: message.signatureStatus,
  });
}

function compareMessages(left: MutableMessage, right: MutableMessage): number {
  const timestampDifference = right.serverTimestamp - left.serverTimestamp;
  return timestampDifference === 0
    ? right.matrixEventId.localeCompare(left.matrixEventId)
    : timestampDifference;
}

function compareReadOnlyEvents(
  left: ReadOnlyFederatedEvent,
  right: ReadOnlyFederatedEvent,
): number {
  const timestampDifference = right.serverTimestamp - left.serverTimestamp;
  return timestampDifference === 0
    ? right.matrixEventId.localeCompare(left.matrixEventId)
    : timestampDifference;
}

function validServerTimestamp(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

function limitProperties(limit: number) {
  return (value: object, context: z.core.$RefinementCtx<object>): void => {
    if (Object.keys(value).length > limit) {
      context.addIssue({ code: 'custom', message: `对象属性不得超过 ${String(limit)} 个。` });
    }
  };
}
