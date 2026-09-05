import { z } from 'zod';

export const commandFailureSchema = z
  .object({
    code: z
      .string()
      .min(1)
      .max(128)
      .regex(/^[a-z0-9._]+$/u),
    retryable: z.boolean(),
  })
  .strict();

const serializedSchema = z.string().trim().min(1).max(1_024);
const envelopeSchema = z.looseObject({ message: serializedSchema });
const aclDenial = /^Command [a-zA-Z0-9_:|.-]+ not allowed by ACL$/u;

export function normalizeCommandFailure(
  error: unknown,
  fallbackCode: string,
): z.output<typeof commandFailureSchema> {
  const parsed = commandFailureSchema.safeParse(error);
  if (parsed.success) return parsed.data;
  const direct = serializedSchema.safeParse(error);
  const envelope = envelopeSchema.safeParse(error);
  const serialized = direct.success ? direct.data : envelope.success ? envelope.data.message : null;
  const fallback = { code: fallbackCode, retryable: true };
  if (serialized === null) return fallback;
  if (aclDenial.test(serialized)) {
    return { code: 'desktop.command.permission_denied', retryable: false };
  }
  try {
    const decoded = commandFailureSchema.safeParse(JSON.parse(serialized) as unknown);
    return decoded.success ? decoded.data : fallback;
  } catch {
    return fallback;
  }
}
