import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneContentClient } from './control-plane-content-client';

const CONTENT_ID = '01990d9e-8400-7000-8000-000000000006';

describe('ControlPlaneContentClient', () => {
  it('构造客户端不会预取票据或正文', () => {
    const fetchImplementation = vi.fn<typeof fetch>();

    new ControlPlaneContentClient({
      baseUrl: 'https://api.agent-room.test',
      fetch: fetchImplementation,
    });

    expect(fetchImplementation).not.toHaveBeenCalled();
  });

  it('显式申请短期票据并保持 Cookie 会话边界', async () => {
    const fetchImplementation = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          expiresAtUnixMs: 1_800_000_000_000,
          ticket: 'short-lived-content-ticket',
        }),
        { headers: { 'Content-Type': 'application/json' } },
      ),
    );
    const client = new ControlPlaneContentClient({
      baseUrl: 'https://api.agent-room.test',
      fetch: fetchImplementation,
    });

    const result = await client.issueReadTicket(CONTENT_ID);

    expect(result).toEqual({
      ok: true,
      value: {
        expiresAtUnixMs: 1_800_000_000_000,
        ticket: 'short-lived-content-ticket',
      },
    });
    expect(fetchImplementation).toHaveBeenCalledWith(
      new URL(`https://api.agent-room.test/content/${CONTENT_ID}/read-tickets`),
      expect.objectContaining({ cache: 'no-store', credentials: 'include', method: 'POST' }),
    );
  });

  it('用票据读取正文，但只返回未信任字节和完整性响应头', async () => {
    const body = new TextEncoder().encode('hello');
    const fetchImplementation = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(body, {
        headers: {
          'Content-Digest': 'sha-256=:q6urq6urq6urq6urq6urq6urq6urq6urq6urq6urq6s=:',
          'Content-Length': String(body.byteLength),
          'Content-Type': 'text/plain',
        },
      }),
    );
    const client = new ControlPlaneContentClient({
      baseUrl: 'https://api.agent-room.test',
      fetch: fetchImplementation,
    });

    const result = await client.download(CONTENT_ID, 'short-lived-content-ticket');

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect([...result.value.bytes]).toEqual([...body]);
    expect(result.value).toMatchObject({
      contentDigest: 'sha-256=:q6urq6urq6urq6urq6urq6urq6urq6urq6urq6urq6s=:',
      contentLength: '5',
      mediaType: 'text/plain',
    });
    expect(fetchImplementation).toHaveBeenCalledWith(
      new URL(`https://api.agent-room.test/content/${CONTENT_ID}/open`),
      expect.objectContaining({
        body: JSON.stringify({ ticket: 'short-lived-content-ticket' }),
        credentials: 'include',
        method: 'POST',
      }),
    );
  });

  it('把服务端错误收敛为稳定错误码而不泄露响应正文', async () => {
    const fetchImplementation = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          code: 'content.not_authorized',
          correlationId: '01990d9e-8400-7000-8000-000000000090',
          retryable: false,
        }),
        { status: 403 },
      ),
    );
    const client = new ControlPlaneContentClient({
      baseUrl: 'https://api.agent-room.test',
      fetch: fetchImplementation,
    });

    await expect(client.issueReadTicket(CONTENT_ID)).resolves.toEqual({
      error: {
        code: 'content.ticket_rejected',
        correlationId: '01990d9e-8400-7000-8000-000000000090',
        retryable: false,
      },
      ok: false,
    });
  });
});
