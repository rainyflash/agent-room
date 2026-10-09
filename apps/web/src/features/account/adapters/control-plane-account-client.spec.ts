import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneAccountClient } from './control-plane-account-client';

const BASE_URL = 'https://app.agent-room.test/_agent-room/api';
const NOW = Date.UTC(2026, 9, 8, 12);

describe('ControlPlaneAccountClient', () => {
  it('导出的数据排好版，文件名带日期', async () => {
    const fetch = vi.fn(() =>
      Promise.resolve(
        Response.json({ schemaVersion: 1, generatedAtUnixMs: NOW, data: { principal: {} } }),
      ),
    );
    const client = new ControlPlaneAccountClient({ baseUrl: BASE_URL, fetch, now: () => NOW });

    const exported = await client.exportData();

    expect(exported).toEqual({
      ok: true,
      value: {
        fileName: 'agent-room-account-2026-10-08.json',
        json: `${JSON.stringify({ schemaVersion: 1, generatedAtUnixMs: NOW, data: { principal: {} } }, null, 2)}\n`,
      },
    });
    const [url, init] = fetch.mock.calls[0] as unknown as [URL, RequestInit];
    expect(url.toString()).toBe(`${BASE_URL}/account/export`);
    expect(init.method).toBe('GET');
    expect(init.credentials).toBe('include');
  });

  it('导出回来的不是账户数据时不存文件', async () => {
    const fetch = vi.fn(() => Promise.resolve(new Response('<html></html>', { status: 200 })));
    const client = new ControlPlaneAccountClient({ baseUrl: BASE_URL, fetch });

    expect(await client.exportData()).toEqual({
      ok: false,
      error: { code: 'account.invalid_export', retryable: false },
    });
  });

  it('删除账户带确认词、同一个请求编号，服务器受理后才算成功', async () => {
    const fetch = vi.fn(() =>
      Promise.resolve(
        Response.json(
          { receipt: 'receipt', progress: { jobId: 'job', stage: 'queued' } },
          { status: 202 },
        ),
      ),
    );
    const client = new ControlPlaneAccountClient({ baseUrl: BASE_URL, fetch });

    const deleted = await client.requestDeletion('018c251e-7b5a-7c7f-8a28-2de53f56a9a3');

    expect(deleted).toEqual({ ok: true, value: undefined });
    const [url, init] = fetch.mock.calls[0] as unknown as [URL, RequestInit];
    expect(url.toString()).toBe(`${BASE_URL}/account`);
    expect(init.method).toBe('DELETE');
    expect(new Headers(init.headers).get('Idempotency-Key')).toBe(
      '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
    );
    expect(JSON.parse(init.body as string)).toEqual({
      confirmation: 'DELETE',
      federationResidualAcknowledged: true,
    });
  });

  it('服务器拒绝时带回错误码', async () => {
    const fetch = vi.fn(() =>
      Promise.resolve(
        Response.json(
          { code: 'authentication.reauthentication_required', correlationId: 'c-1' },
          { status: 401 },
        ),
      ),
    );
    const client = new ControlPlaneAccountClient({ baseUrl: BASE_URL, fetch });

    expect(await client.requestDeletion('018c251e-7b5a-7c7f-8a28-2de53f56a9a3')).toEqual({
      ok: false,
      error: {
        code: 'authentication.reauthentication_required',
        correlationId: 'c-1',
        retryable: false,
      },
    });
  });
});
