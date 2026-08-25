import { z } from 'zod';

import {
  matrixMessagePreviewEventType,
  matrixMessageRevisionEventType,
  matrixModerationNoticeEventType,
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
  type MessageSignatureStatus,
  type RoomMessageSignal,
} from '@/features/messages/domain/message';
import { err, ok } from '@/shared/result';

const MAX_PROJECTED_MESSAGES = 200;
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
const actorSchema = z
  .looseObject({
    agent: z
      .looseObject({
        agentId: uuidV7Schema,
        avatarUrl: z
          .string()
          .max(2_048)
          .regex(/^https:\/\//u)
          .optional(),
        displayName: z.string().min(1).max(80),
        matrixUserId: matrixUserIdSchema,
      })
      .superRefine(limitProperties(16)),
    instanceId: uuidV7Schema,
    provenance: z.enum(messageProvenances),
  })
  .superRefine(limitProperties(12));
const contentSchema = z
  .looseObject({
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
    summary: z.string().min(1).max(500),
    title: z.string().min(1).max(120),
  })
  .superRefine(limitProperties(16));
const relationSchema = z
  .looseObject({
    kind: z.literal('reply'),
    targetMessageId: uuidV7Schema,
  })
  .superRefine(limitProperties(8));
const commonEventShape = {
  actor: actorSchema,
  correlationId: z.uuid(),
  createdAt: z.iso.datetime({ offset: true }),
  id: uuidV7Schema,
  roomId: matrixRoomIdSchema,
  schemaVersion: z.literal('1.0'),
  signature: z
    .string()
    .min(43)
    .max(128)
    .regex(/^[A-Za-z0-9_-]+$/u),
};
const previewEventSchema = z
  .looseObject({
    ...commonEventShape,
    content: contentSchema,
    eventType: z.literal(matrixMessagePreviewEventType),
    preview: previewSchema,
    relation: relationSchema.optional(),
  })
  .superRefine(limitProperties(24));
const revisionEventSchema = z
  .looseObject({
    ...commonEventShape,
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
    targetEventId: z.string().min(2).max(1_024).regex(/^\$[^\u0000-\u001f\u007f]+$/u),
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
  const seenMatrixEventIds = new Set<string>();

  for (const timelineEvent of room.timelineEvents) {
    const eventId = timelineEvent.eventId;
    if (eventId === undefined || seenMatrixEventIds.has(eventId)) {
      continue;
    }
    seenMatrixEventIds.add(eventId);
    if (timelineEvent.type === matrixMessagePreviewEventType) {
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
    if (timelineEvent.type === matrixMessageRevisionEventType) {
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
    roomId: room.roomId,
  });
}

function parseModerationNotice(
  event: MatrixMessageTimelineEvent,
): ParsedModerationNotice | null {
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
    parsed.data.actor.agent.matrixUserId !== event.sender ||
    parsed.data.preview.contentType !== parsed.data.content.mediaType ||
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
    parsed.data.actor.agent.matrixUserId !== event.sender
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
    sender: parsed.data.actor.agent.matrixUserId,
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
  const agent = event.actor.agent;
  return Object.freeze({
    agentId: agent.agentId,
    ...(agent.avatarUrl === undefined ? {} : { avatarUrl: agent.avatarUrl }),
    displayName: agent.displayName,
    instanceId: event.actor.instanceId,
    matrixUserId: agent.matrixUserId,
    provenance: event.actor.provenance,
  });
}

function toContent(content: z.output<typeof contentSchema>): MessageContentReference {
  return Object.freeze({
    contentId: content.contentId,
    digestSha256: content.digestSha256,
    mediaType: content.mediaType,
    sizeBytes: content.sizeBytes,
  });
}

function toPreview(preview: z.output<typeof previewSchema>): MessagePreview {
  return Object.freeze({
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
