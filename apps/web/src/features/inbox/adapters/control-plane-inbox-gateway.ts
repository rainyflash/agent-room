import { z } from 'zod';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok } from '@/shared/result';
import { matrixRoomIdSchema } from '@/shared/validation/identifiers';
import type { InboxIndexGateway } from '../domain/inbox';

const responseSchema = z
  .object({
    accountId: z.string().min(4).max(255),
    rooms: z
      .array(
        z
          .object({
            catalogId: z.uuid(),
            roomId: matrixRoomIdSchema,
            name: z.string().min(1).max(1000),
            direct: z.boolean(),
          })
          .strict(),
      )
      .max(1000),
    handoffs: z
      .array(
        z
          .object({
            handoffId: z.uuid(),
            roomId: matrixRoomIdSchema,
            messageId: z.uuid(),
            agentName: z.string().max(160),
            status: z.enum(['queued', 'delivered', 'failed']),
            createdAtUnixMs: z.number().int().nonnegative(),
          })
          .strict(),
      )
      .max(200),
    limited: z.boolean(),
  })
  .strict();

export class ControlPlaneInboxGateway implements InboxIndexGateway {
  constructor(
    private readonly options: { readonly baseUrl: string; readonly fetch: typeof globalThis.fetch },
  ) {}
  readonly read: InboxIndexGateway['read'] = async (accountId) => {
    try {
      const response = await this.options.fetch(
        controlPlaneEndpoint(this.options.baseUrl, '/inbox'),
        { credentials: 'include', cache: 'no-store', signal: AbortSignal.timeout(15000) },
      );
      if (!response.ok) return err({ code: 'inbox.unavailable', retryable: true });
      const parsed = responseSchema.safeParse(await response.json());
      if (!parsed.success) return err({ code: 'inbox.invalid_response', retryable: false });
      if (parsed.data.accountId !== accountId)
        return err({ code: 'inbox.session_changed', retryable: true });
      return ok({
        accountId,
        rooms: parsed.data.rooms,
        handoffs: parsed.data.handoffs,
        limited: parsed.data.limited,
      });
    } catch {
      return err({ code: 'inbox.unavailable', retryable: true });
    }
  };
}
