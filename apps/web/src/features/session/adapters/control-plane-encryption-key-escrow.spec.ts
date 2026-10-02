import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneEncryptionKeyEscrow } from './control-plane-encryption-key-escrow';
import { CrossSigningUploadFailure } from './matrix-device-signing';

const BASE = 'https://app.agent-room.test/_agent-room/api';
const KEY = new Uint8Array(32).fill(5);
const KEY_BASE64 = btoa(String.fromCharCode(...KEY));

describe('ControlPlaneEncryptionKeyEscrow', () => {
  it('取钥匙：带登录 Cookie、不缓存，解出 32 字节', async () => {
    const fetch = respond(200, { schemaVersion: 1, keyId: 'KEY1', key: KEY_BASE64 });

    await expect(escrow(fetch).fetch()).resolves.toEqual({ key: KEY, keyId: 'KEY1' });

    const [url, init] = fetch.mock.calls[0] ?? [];
    expect(href(url)).toBe(`${BASE}/account/encryption-key`);
    expect(init).toMatchObject({ cache: 'no-store', credentials: 'include', method: 'GET' });
  });

  it('控制面明说还没有钥匙才算没有；没带这个错误码的 404 当作失败', async () => {
    await expect(
      escrow(respond(404, { code: 'account.encryption_key_missing' })).fetch(),
    ).resolves.toBeNull();
    await expect(escrow(respond(404, 'Not Found')).fetch()).rejects.toThrow();
    await expect(escrow(respond(503, { code: 'account.unavailable' })).fetch()).rejects.toThrow();
  });

  it('钥匙长度不对就不用', async () => {
    const short = btoa('short');
    await expect(
      escrow(respond(200, { schemaVersion: 1, keyId: 'KEY1', key: short })).fetch(),
    ).rejects.toThrow();
  });

  it('存钥匙用 PUT 送 base64；代传签名公钥用 POST 送原样的公钥；失败都抛出', async () => {
    const fetch = respond(204);
    const client = escrow(fetch);
    const keys = { master_key: { user_id: '@rainy:agent-room.test', usage: ['master'] } };

    await client.store({ key: KEY, keyId: 'KEY1' });
    await client.replaceCrossSigningKeys(keys);

    const [storeUrl, storeInit] = fetch.mock.calls[0] ?? [];
    expect(href(storeUrl)).toBe(`${BASE}/account/encryption-key`);
    expect(storeInit?.method).toBe('PUT');
    expect(JSON.parse(typeof storeInit?.body === 'string' ? storeInit.body : '')).toEqual({
      key: KEY_BASE64,
      keyId: 'KEY1',
    });
    const [resetUrl, resetInit] = fetch.mock.calls[1] ?? [];
    expect(href(resetUrl)).toBe(`${BASE}/account/encryption-reset`);
    expect(resetInit).toMatchObject({ credentials: 'include', method: 'POST' });
    expect(JSON.parse(typeof resetInit?.body === 'string' ? resetInit.body : '')).toEqual(keys);

    await expect(escrow(respond(403)).store({ key: KEY, keyId: 'KEY1' })).rejects.toThrow();
  });

  it('代传被拒绝（公钥不对、太勤）不该重试；控制面暂时不可用、连不上可以再试', async () => {
    for (const [status, retryable] of [
      [400, false],
      [429, false],
      [503, true],
    ] as const) {
      const failure = await escrow(respond(status))
        .replaceCrossSigningKeys({})
        .catch((error: unknown) => error);
      expect(failure).toBeInstanceOf(CrossSigningUploadFailure);
      expect((failure as CrossSigningUploadFailure).retryable).toBe(retryable);
    }
    const offline = vi.fn<typeof globalThis.fetch>(() => Promise.reject(new TypeError('offline')));
    const failure = await escrow(offline)
      .replaceCrossSigningKeys({})
      .catch((error: unknown) => error);
    expect((failure as CrossSigningUploadFailure).retryable).toBe(true);
  });
});

function escrow(fetch: typeof globalThis.fetch) {
  return new ControlPlaneEncryptionKeyEscrow({ baseUrl: BASE, fetch });
}

function respond(status: number, body?: unknown) {
  return vi.fn<typeof globalThis.fetch>(() =>
    Promise.resolve(
      new Response(
        body === undefined || status === 204
          ? null
          : typeof body === 'string'
            ? body
            : JSON.stringify(body),
        { status },
      ),
    ),
  );
}

function href(input: unknown): string {
  return input instanceof URL ? input.href : typeof input === 'string' ? input : '';
}
