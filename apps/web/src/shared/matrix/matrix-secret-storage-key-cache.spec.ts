import { describe, expect, it } from 'vitest';

import { MatrixSecretStorageKeyCache } from '@/shared/matrix/matrix-secret-storage-key-cache';

describe('MatrixSecretStorageKeyCache', () => {
  it('只向 Matrix 回调提供当前进程已经解锁且被请求的密钥', async () => {
    const cache = new MatrixSecretStorageKeyCache();
    const original = Uint8Array.from([1, 2, 3]);
    cache.unlock('known', original);
    original.fill(9);

    await expect(
      cache.callbacks.getSecretStorageKey?.(
        { keys: { unknown: {} as never } },
        'm.cross_signing.master',
      ),
    ).resolves.toBeNull();
    await expect(
      cache.callbacks.getSecretStorageKey?.(
        { keys: { known: {} as never } },
        'm.cross_signing.master',
      ),
    ).resolves.toEqual(['known', Uint8Array.from([1, 2, 3])]);
  });

  it('清理后撤销后续访问', async () => {
    const cache = new MatrixSecretStorageKeyCache();
    cache.unlock('known', Uint8Array.from([1, 2, 3]));

    cache.clear();

    await expect(
      cache.callbacks.getSecretStorageKey?.({ keys: { known: {} as never } }, 'm.megolm_backup.v1'),
    ).resolves.toBeNull();
  });
});
