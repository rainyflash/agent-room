import { z } from 'zod';
import { createAutomationGrantInputSchema } from '@/features/automation/domain/automation-grant';
import { safeInternalPath } from '@/shared/browser/window-browser-gateway';
import { err, ok, type Result } from '@/shared/result';

export const automationGrantDraftInputSchema = createAutomationGrantInputSchema.omit({
  impactAcknowledged: true,
});
export type AutomationGrantDraftInput = z.output<typeof automationGrantDraftInputSchema>;

const draftSchema = z
  .object({
    input: automationGrantDraftInputSchema,
    principalId: z.uuid(),
    returnPath: z
      .string()
      .max(2048)
      .refine((path) => safeInternalPath(path) !== null),
    savedAt: z.number().int().nonnegative(),
  })
  .strict();
type PendingDraft = z.output<typeof draftSchema>;
const storageKey = 'agent-room.automation-draft.v1';
const lifetimeMs = 60 * 60 * 1000;

export function readAutomationGrantDraft(
  principalId: string,
): Result<PendingDraft | null, 'storage'> {
  try {
    const raw = window.sessionStorage.getItem(storageKey);
    if (raw === null) return ok(null);
    const decoded: unknown = JSON.parse(raw);
    const parsed = draftSchema.safeParse(decoded);
    if (!parsed.success) return err('storage');
    const draft = parsed.data;
    const age = Date.now() - draft.savedAt;
    return ok(draft.principalId === principalId && age >= 0 && age <= lifetimeMs ? draft : null);
  } catch {
    return err('storage');
  }
}

export function saveAutomationGrantDraft(
  principalId: string,
  input: AutomationGrantDraftInput,
): Result<void, 'storage'> {
  try {
    const candidate = draftSchema.safeParse({
      // Persist settings only; acknowledgement must be given again after verification.
      input: automationGrantDraftInputSchema.strip().parse(input),
      principalId,
      returnPath: `${window.location.pathname}${window.location.search}${window.location.hash}`,
      savedAt: Date.now(),
    });
    if (!candidate.success) return err('storage');
    window.sessionStorage.setItem(storageKey, JSON.stringify(candidate.data));
    return ok(undefined);
  } catch {
    return err('storage');
  }
}

export function clearAutomationGrantDraft(
  principalId: string,
  catalogId: string,
): Result<void, 'storage'> {
  const result = readAutomationGrantDraft(principalId);
  if (!result.ok) return result;
  if (result.value?.input.roomCatalogId !== catalogId) return ok(undefined);
  try {
    window.sessionStorage.removeItem(storageKey);
    return ok(undefined);
  } catch {
    return err('storage');
  }
}
