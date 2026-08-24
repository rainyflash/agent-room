import { describe, expect, it, vi } from 'vitest';

import { BrowserContentVerifier } from './browser-content-verifier';
import type { DownloadedContent } from '@/features/messages/domain/content';
import type { MessageContentReference } from '@/features/messages/domain/message';

const digestBytes = new Uint8Array(32).fill(0xab);
const digestHex = 'ab'.repeat(32);
const digestHeader = `sha-256=:${toBase64(digestBytes)}:`;

describe('BrowserContentVerifier', () => {
  it('同时校验长度、响应摘要、实际 SHA-256 与媒体类型后才解码文本', async () => {
    const digest = vi.fn(() => Promise.resolve(digestBytes.buffer));
    const verifier = new BrowserContentVerifier({ digest });
    const body = new TextEncoder().encode('# 安全正文');

    const result = await verifier.verify(download(body), reference(body.byteLength));

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toMatchObject({
      digestSha256: digestHex,
      mediaType: 'text/markdown',
      mode: 'text',
      text: '# 安全正文',
    });
    expect(digest).toHaveBeenCalledOnce();
  });

  it.each([
    {
      code: 'content.length_mismatch',
      mutate: (value: DownloadedContent) => ({ ...value, contentLength: '999' }),
    },
    {
      code: 'content.digest_mismatch',
      mutate: (value: DownloadedContent) => ({ ...value, contentDigest: null }),
    },
    {
      code: 'content.media_type_mismatch',
      mutate: (value: DownloadedContent) => ({ ...value, mediaType: 'text/plain' }),
    },
  ] as const)('在 $code 时失败关闭', async ({ code, mutate }) => {
    const body = new TextEncoder().encode('正文');
    const verifier = new BrowserContentVerifier({
      digest: () => Promise.resolve(digestBytes.buffer),
    });

    const result = await verifier.verify(mutate(download(body)), reference(body.byteLength));

    expect(result).toEqual({ error: { code, retryable: false }, ok: false });
  });

  it('响应头合法但实际字节摘要不同仍拒绝内容', async () => {
    const body = new TextEncoder().encode('正文');
    const verifier = new BrowserContentVerifier({
      digest: () => Promise.resolve(new Uint8Array(32).fill(0xcd).buffer),
    });

    await expect(verifier.verify(download(body), reference(body.byteLength))).resolves.toEqual({
      error: { code: 'content.digest_mismatch', retryable: false },
      ok: false,
    });
  });

  it('附件只进入显式下载模式，不尝试解释或执行', async () => {
    const body = new Uint8Array([0, 1, 2, 3]);
    const verifier = new BrowserContentVerifier({
      digest: () => Promise.resolve(digestBytes.buffer),
    });
    const downloaded = { ...download(body), mediaType: 'application/octet-stream' };
    const expected = { ...reference(body.byteLength), mediaType: 'application/octet-stream' };

    const result = await verifier.verify(downloaded, expected);

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value.mode).toBe('download');
      expect(result.value.text).toBeUndefined();
    }
  });
});

function download(bytes: Uint8Array): DownloadedContent {
  return {
    bytes,
    contentDigest: digestHeader,
    contentLength: String(bytes.byteLength),
    mediaType: 'text/markdown; charset=utf-8',
  };
}

function reference(sizeBytes: number): MessageContentReference {
  return {
    contentId: '01990d9e-8400-7000-8000-000000000006',
    digestSha256: digestHex,
    mediaType: 'text/markdown',
    sizeBytes,
  };
}

function toBase64(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes));
}
