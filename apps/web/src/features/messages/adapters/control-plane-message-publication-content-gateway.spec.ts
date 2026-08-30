import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneMessagePublicationContentGateway } from './control-plane-message-publication-content-gateway';

const submissionId = '01990d9e-8400-7000-8000-000000000003';
const contentId = '01990d9e-8400-7000-8000-000000000004';
const roomId = '!public:agent-room.test';
const digestSha256 = 'a'.repeat(64);

describe('ControlPlaneMessagePublicationContentGateway', () => {
  it('按 UUIDv7 幂等键创建、上传并绑定正文', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValueOnce(jsonResponse(beginResponse(), 201))
      .mockResolvedValueOnce(jsonResponse(completeResponse()))
      .mockResolvedValueOnce(
        jsonResponse({
          accessMode: 'room_member',
          alreadyBound: false,
          contentId,
          matrixEventId: '$event',
          matrixRoomId: roomId,
        }),
      );
    const gateway = new ControlPlaneMessagePublicationContentGateway({
      baseUrl: 'https://api.room.test',
      fetch,
    });
    const bytes = new Uint8Array([104, 101, 108, 108, 111]);

    const uploaded = await gateway.upload({
      body: { bytes, digestSha256 },
      mediaType: 'text/markdown',
      roomId,
      submissionId,
    });
    const bound = await gateway.bind({ contentId, matrixEventId: '$event', roomId });

    expect(uploaded).toEqual({
      ok: true,
      value: { contentId, digestSha256, mediaType: 'text/markdown', sizeBytes: 5 },
    });
    expect(bound).toEqual({ ok: true, value: undefined });
    expect(fetch).toHaveBeenCalledTimes(3);
    const firstRequest = fetch.mock.calls[0]?.[1];
    expect(firstRequest).toMatchObject({
      cache: 'no-store',
      credentials: 'include',
      method: 'POST',
    });
    expect(new Headers(firstRequest?.headers).get('Idempotency-Key')).toBe(submissionId);
    expect(firstRequest?.body).toBe(
      JSON.stringify({
        accessMode: 'room_member',
        byteLength: 5,
        encryptionMode: 'server_side',
        matrixRoomId: roomId,
        mediaType: 'text/markdown',
        sha256: digestSha256,
      }),
    );
    expect(fetch.mock.calls[1]?.[1]).toMatchObject({
      body: bytes,
      credentials: 'include',
      method: 'PUT',
    });
  });

  it('拒绝服务端返回的摘要错配并把网络中断映射为可重试边界', async () => {
    const mismatch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(jsonResponse({ ...beginResponse(), sha256: 'b'.repeat(64) }, 201));
    const request = {
      body: { bytes: new Uint8Array([1]), digestSha256 },
      mediaType: 'text/plain' as const,
      roomId,
      submissionId,
    };

    await expect(
      new ControlPlaneMessagePublicationContentGateway({
        baseUrl: 'https://api.room.test',
        fetch: mismatch,
      }).upload(request),
    ).resolves.toEqual({
      error: { code: 'publication.content_rejected', retryable: false },
      ok: false,
    });

    const offline = vi.fn<typeof globalThis.fetch>().mockRejectedValue(new TypeError('offline'));
    await expect(
      new ControlPlaneMessagePublicationContentGateway({
        baseUrl: 'https://api.room.test',
        fetch: offline,
      }).upload(request),
    ).resolves.toEqual({
      error: { code: 'publication.content_rejected', retryable: true },
      ok: false,
    });
  });
});

function beginResponse() {
  return {
    accessMode: 'room_member',
    byteLength: 5,
    contentId,
    created: true,
    createdAtUnixMs: 1_700_000_000_000,
    encryptionMode: 'server_side',
    expiresAtUnixMs: null,
    lifecycleState: 'uploading',
    matrixRoomId: roomId,
    mediaType: 'text/markdown',
    scanState: 'pending',
    sha256: digestSha256,
  };
}

function completeResponse() {
  return {
    alreadyActive: false,
    byteLength: 5,
    contentId,
    createdAtUnixMs: 1_700_000_000_000,
    encryptionMode: 'server_side',
    expiresAtUnixMs: null,
    lifecycleState: 'active',
    mediaType: 'text/markdown',
    scanState: 'clean',
    sha256: digestSha256,
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    headers: { 'Content-Type': 'application/json' },
    status,
  });
}
