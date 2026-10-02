// @vitest-environment jsdom

import { describe, expect, it, vi } from 'vitest';

import { routeCrossSigningUpload } from './cross-signing-upload-route';
import { CONTROL_PLANE_UPLOAD, CrossSigningUploadFailure } from './matrix-device-signing';

const UPLOAD = 'https://matrix.agent-room.test/_matrix/client/v3/keys/device_signing/upload';
const KEYS = {
  master_key: { user_id: '@rainy:agent-room.test', usage: ['master'], keys: { 'ed25519:m': 'm' } },
};

describe('routeCrossSigningUpload', () => {
  it('带着我们认证标记的签名公钥上传改送控制面，去掉认证，回 200', async () => {
    const network = vi.fn<typeof fetch>();
    const escrow = { replaceCrossSigningKeys: vi.fn(() => Promise.resolve()) };
    const route = routeCrossSigningUpload(network, escrow);

    const response = await route(UPLOAD, {
      body: JSON.stringify({ ...KEYS, auth: { type: CONTROL_PLANE_UPLOAD } }),
      method: 'POST',
    });

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toEqual({});
    expect(escrow.replaceCrossSigningKeys).toHaveBeenCalledWith(KEYS);
    expect(network).not.toHaveBeenCalled();
  });

  it('别的请求原样发给 Matrix：别的路径、不是 POST、没带标记或带的是别的认证', async () => {
    const network = vi.fn<typeof fetch>(() => Promise.resolve(new Response('{}')));
    const escrow = { replaceCrossSigningKeys: vi.fn(() => Promise.resolve()) };
    const route = routeCrossSigningUpload(network, escrow);
    const requests: [string, RequestInit][] = [
      ['https://matrix.agent-room.test/_matrix/client/v3/sync', { method: 'GET' }],
      [UPLOAD, { method: 'GET' }],
      [UPLOAD, { body: JSON.stringify(KEYS), method: 'POST' }],
      [
        UPLOAD,
        { body: JSON.stringify({ ...KEYS, auth: { type: 'm.login.password' } }), method: 'POST' },
      ],
      [UPLOAD, { body: 'not json', method: 'POST' }],
    ];

    for (const [url, init] of requests) await route(url, init);

    expect(network).toHaveBeenCalledTimes(requests.length);
    expect(escrow.replaceCrossSigningKeys).not.toHaveBeenCalled();
  });

  it('控制面拒绝时回 400（matrix-js-sdk 不重试），暂时不可用时回 503（它会再试）', async () => {
    for (const [failure, status] of [
      [new CrossSigningUploadFailure(false, 'rejected'), 400],
      [new CrossSigningUploadFailure(true, 'unavailable'), 503],
      [new Error('unexpected'), 503],
    ] as const) {
      const route = routeCrossSigningUpload(vi.fn<typeof fetch>(), {
        replaceCrossSigningKeys: vi.fn(() => Promise.reject(failure)),
      });
      const response = await route(UPLOAD, {
        body: JSON.stringify({ ...KEYS, auth: { type: CONTROL_PLANE_UPLOAD } }),
        method: 'POST',
      });
      expect(response.status).toBe(status);
      const body = (await response.json()) as { errcode?: string };
      expect(body.errcode).toBe(status === 400 ? 'M_FORBIDDEN' : 'M_UNKNOWN');
    }
  });
});
