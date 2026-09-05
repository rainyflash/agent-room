import { contentEncryptionSchema } from './content-encryption-schema';
import { conversationSchema } from '@/features/conversation/adapters/conversation-schema';
import { z } from 'zod';

import type {
  MessagePublicationFailure,
  MessageSubmissionJournal,
  MessageSubmissionRecord,
  ProtectedMessageBody,
} from '@/features/messages/domain/publication';
import { err, ok } from '@/shared/result';

const uuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u);
const contentSchema = z
  .object({
    encryption: contentEncryptionSchema.optional(),
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
    displayName: z
      .string()
      .min(1)
      .max(160)
      .refine((value) => Array.from(value).length <= 80),
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
        conversation: conversationSchema.optional(),
        contentType: z.enum(['text/markdown', 'text/plain']),
        language: z.string().min(2).max(35).optional(),
        riskFlags: z.array(z.string().min(1).max(64)).max(16),
        sensitivity: z.enum(['normal', 'sensitive', 'restricted']),
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

  readonly #bodies = new Map<string, ProtectedMessageBody>();

  releaseBody(scope: string, submissionId: string): void {
    const record = this.#memory.get(submissionId);
    if (record === undefined) return;
    if (this.#storage !== null) {
      try {
        // 只有提交记录已经可靠持久化，才能删除上传前保留的密文副本。
        if (this.#storage.getItem(`${storagePrefix}${submissionId}`) !== JSON.stringify(record))
          return;
        this.#storage.removeItem(`${storagePrefix}body.${scope}`);
      } catch {
        // 保留副本支持恢复，清理失败不会改变已经提交的消息结果。
        return;
      }
    }
    this.#bodies.delete(scope);
  }

  readBody(scope: string) {
    const cached = this.#bodies.get(scope);
    if (cached !== undefined) return ok(cached);
    try {
      const text = this.#storage?.getItem(`${storagePrefix}body.${scope}`);
      if (text === null || text === undefined) return ok(null);
      const parsed = z
        .object({
          bytes: z.array(z.number().int().min(0).max(255)).max(25 * 1024 * 1024),
          digestSha256: z.string().regex(/^[a-f0-9]{64}$/u),
          encryption: contentEncryptionSchema,
        })
        .strict()
        .safeParse(JSON.parse(text));
      if (!parsed.success) return err(persistenceFailure(false));
      const value = {
        body: { bytes: Uint8Array.from(parsed.data.bytes), digestSha256: parsed.data.digestSha256 },
        encryption: parsed.data.encryption,
      };
      this.#bodies.set(scope, value);
      return ok(value);
    } catch {
      return err(persistenceFailure(true));
    }
  }

  writeBody(scope: string, value: ProtectedMessageBody) {
    this.#bodies.set(scope, value);
    if (value.encryption === undefined || this.#storage === null) return ok(undefined);
    try {
      this.#storage.setItem(
        `${storagePrefix}body.${scope}`,
        JSON.stringify({
          bytes: [...value.body.bytes],
          digestSha256: value.body.digestSha256,
          encryption: value.encryption,
        }),
      );
      return ok(undefined);
    } catch {
      // 加密草稿必须先可靠保存，防止重新加载后同一个上传幂等键对应不同密文。
      return err(persistenceFailure(false));
    }
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
  const { encryption, ...contentFields } = record.content;
  const content = Object.freeze({
    ...contentFields,
    ...(encryption === undefined ? {} : { encryption }),
  });
  const { encryption: eventEncryption, ...eventFields } = record.event.content;
  const eventContent = Object.freeze({
    ...eventFields,
    ...(eventEncryption === undefined ? {} : { encryption: eventEncryption }),
  });
  const actor = Object.freeze({ ...record.event.actor });
  const preview = Object.freeze({
    ...(record.event.preview.conversation === undefined
      ? {}
      : { conversation: record.event.preview.conversation }),
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
