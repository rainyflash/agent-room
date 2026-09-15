import { z } from 'zod';
import { controlPlaneEndpoint } from '@/shared/http/control-plane-endpoint';
import { err, ok } from '@/shared/result';
import {
  receptionRecordSchema,
  type ReceptionOwnershipGateway,
} from '../domain/reception-ownership';
export class ReceptionOwnershipClient implements ReceptionOwnershipGateway {
  constructor(
    private readonly options: { readonly baseUrl: string; readonly fetch: typeof globalThis.fetch },
  ) {}
  readonly list: ReceptionOwnershipGateway['list'] = () =>
    this.request(
      '/receptions',
      z
        .object({ receptions: z.array(receptionRecordSchema).max(128), limited: z.boolean() })
        .strict(),
    );
  readonly transfer: ReceptionOwnershipGateway['transfer'] = (request) =>
    this.request('/receptions/transfer', receptionRecordSchema, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(request),
    });
  private async request<T>(path: string, schema: z.ZodType<T>, init: RequestInit = {}) {
    try {
      const response = await this.options.fetch(controlPlaneEndpoint(this.options.baseUrl, path), {
        ...init,
        credentials: 'include',
        cache: 'no-store',
        signal: AbortSignal.timeout(15000),
      });
      if (!response.ok)
        return err({
          code: response.status === 409 ? 'reception.execution_conflict' : 'reception.unavailable',
          retryable: true,
        });
      const parsed = schema.safeParse(await response.json());
      return parsed.success
        ? ok(parsed.data)
        : err({ code: 'reception.invalid_response', retryable: false });
    } catch {
      return err({ code: 'reception.unavailable', retryable: true });
    }
  }
}
