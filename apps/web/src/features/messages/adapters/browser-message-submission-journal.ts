import { z } from 'zod';

import type {
  MessagePublicationFailure,
  MessageSubmissionJournal,
  MessageSubmissionRecord,
} from '@/features/messages/domain/publication';
import { err, ok } from '@/shared/result';

const uuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u);
const contentSchema = z
  .object({
    contentId: uuidV7Schema,
    digestSha256: z.string().regex(/^[0-9a-f]{64}$/u),
    mediaType: z.enum(['text/markdown', 'text/plain']),
    sizeBytes: z
      .number()
      .int()
      .positive()
      .max(25 * 1_024 * 1_024),
  })
  .strict();
const eventContentSchema = contentSchema.extend({ fetchMode: z.literal('on_demand') }).strict();
const actorSchema = z
  .object({
    displayName: z.string().min(1).max(80),
    kind: z.literal('human'),
    matrixUserId: z.string().regex(/^@[^:]+:[^:]+$/u),
    principalId: uuidV7Schema,
  })
  .strict();
const eventSchema = z
  .object({
    actor: actorSchema,
    content: eventContentSchema,
    correlationId: z.uuid(),
    createdAt: z.iso.datetime({ offset: true }),
    eventType: z.literal('io.github.rainyflash.agentroom.message.preview.v2'),
    id: uuidV7Schema,
    preview: z
      .object({
        contentType: z.enum(['text/markdown', 'text/plain']),
        language: z.string().min(2).max(35).optional(),
        riskFlags: z.array(z.string().min(1).max(64)).max(16),
        sensitivity: z.enum(['normal', 'sensitive', 'restricted']),
        summary: z.string().min(1).max(500),
        title: z.string().min(1).max(120),
      })
      .strict(),
    relation: z
      .object({ kind: z.literal('reply'), targetMessageId: uuidV7Schema })
      .strict()
      .optional(),
    roomId: z.string().regex(/^![^:]+:[^:]+$/u),
    schemaVersion: z.literal('2.0'),
  })
  .strict();
const recordSchema = z
  .object({
    content: contentSchema,
    event: eventSchema,
    fingerprint: z.string().regex(/^[0-9a-f]{64}$/u),
    matrixEventId: z.string().min(2).max(1_024).startsWith('$').optional(),
    roomId: z.string().regex(/^![^:]+:[^:]+$/u),
    submissionId: uuidV7Schema,
    transactionId: z.string().min(20).max(256),
  })
  .strict();

const storagePrefix = 'agent-room.message-submission.v2.';

export class BrowserMessageSubmissionJournal implements MessageSubmissionJournal {
  readonly #memory = new Map<string, MessageSubmissionRecord>();
  readonly #storage: Storage | null;

  constructor(storage: Storage | null) {
    this.#storage = storage;
  }

  read(submissionId: string) {
    const memoryRecord = this.#memory.get(submissionId);
    if (memoryRecord !== undefined) {
      return ok(memoryRecord);
    }
    if (this.#storage === null) {
      return ok(null);
    }
    try {
      const serialized = this.#storage.getItem(`${storagePrefix}${submissionId}`);
      if (serialized === null) {
        return ok(null);
      }
      const parsed = recordSchema.safeParse(JSON.parse(serialized));
      if (!parsed.success || parsed.data.submissionId !== submissionId) {
        return err(persistenceFailure(false));
      }
      const record = freezeRecord(parsed.data);
      this.#memory.set(submissionId, record);
      return ok(record);
    } catch {
      return err(persistenceFailure(true));
    }
  }

  write(record: MessageSubmissionRecord) {
    const parsed = recordSchema.safeParse(record);
    if (!parsed.success) {
      return err(persistenceFailure(false));
    }
    const frozen = freezeRecord(parsed.data);
    this.#memory.set(record.submissionId, frozen);
    if (this.#storage === null) {
      return ok(undefined);
    }
    try {
      this.#storage.setItem(`${storagePrefix}${record.submissionId}`, JSON.stringify(frozen));
      return ok(undefined);
    } catch {
      // 内存日志仍可保证当前会话恢复；浏览器存储不可用不应阻断已登录用户发消息。
      return ok(undefined);
    }
  }
}

function freezeRecord(record: z.output<typeof recordSchema>): MessageSubmissionRecord {
  const content = Object.freeze({ ...record.content });
  const eventContent = Object.freeze({ ...record.event.content });
  const actor = Object.freeze({ ...record.event.actor });
  const preview = Object.freeze({
    contentType: record.event.preview.contentType,
    ...(record.event.preview.language === undefined
      ? {}
      : { language: record.event.preview.language }),
    riskFlags: Object.freeze([...record.event.preview.riskFlags]),
    sensitivity: record.event.preview.sensitivity,
    summary: record.event.preview.summary,
    title: record.event.preview.title,
  });
  const event = Object.freeze({
    actor,
    content: eventContent,
    correlationId: record.event.correlationId,
    createdAt: record.event.createdAt,
    eventType: record.event.eventType,
    id: record.event.id,
    preview,
    ...(record.event.relation === undefined
      ? {}
      : { relation: Object.freeze({ ...record.event.relation }) }),
    roomId: record.event.roomId,
    schemaVersion: record.event.schemaVersion,
  });
  return Object.freeze({
    content,
    event,
    fingerprint: record.fingerprint,
    ...(record.matrixEventId === undefined ? {} : { matrixEventId: record.matrixEventId }),
    roomId: record.roomId,
    submissionId: record.submissionId,
    transactionId: record.transactionId,
  });
}

function persistenceFailure(retryable: boolean): MessagePublicationFailure {
  return Object.freeze({ code: 'publication.persistence_failed', retryable });
}
