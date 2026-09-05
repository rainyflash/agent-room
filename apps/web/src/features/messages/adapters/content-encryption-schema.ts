import { z } from 'zod';

export const contentEncryptionSchema = z
  .object({
    algorithm: z.literal('io.github.rainyflash.agentroom.content.aes-256-gcm.v1'),
    contextId: z
      .string()
      .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u),
    keyBase64Url: z.string().regex(/^[A-Za-z0-9_-]{43}$/u),
    nonceBase64Url: z.string().regex(/^[A-Za-z0-9_-]{16}$/u),
    plaintextSizeBytes: z
      .number()
      .int()
      .positive()
      .max(25 * 1024 * 1024 - 16),
  })
  .strict();
