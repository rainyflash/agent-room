import { describe, expect, it, vi } from 'vitest';

import { BrowserMessageBodyPreparer } from './browser-message-body-preparer';

describe('BrowserMessageBodyPreparer', () => {
  it('用 UTF-8 字节生成小写 SHA-256 并拒绝异常长度摘要', async () => {
    const digest = vi.fn(() =>
      Promise.resolve(Uint8Array.from({ length: 32 }, (_, index) => index).buffer),
    );
    const preparer = new BrowserMessageBodyPreparer({ digest });

    const result = await preparer.prepare('你好');

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect([...result.value.bytes]).toEqual([228, 189, 160, 229, 165, 189]);
      expect(result.value.digestSha256).toBe(
        '000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f',
      );
    }

    const invalid = new BrowserMessageBodyPreparer({
      digest: () => Promise.resolve(new Uint8Array(31).buffer),
    });
    await expect(invalid.prepare('x')).resolves.toEqual({
      error: { code: 'publication.unexpected_failure', retryable: false },
      ok: false,
    });
  });
});
