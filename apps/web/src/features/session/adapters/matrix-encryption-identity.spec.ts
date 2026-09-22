import type { MatrixClient } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import { ensureFirstEncryptionIdentity } from './matrix-encryption-identity';

describe('ensureFirstEncryptionIdentity', () => {
  it('从未建立过加密身份的账户自动建立，不需要去安全中心', async () => {
    const identity = cryptoWith(false);

    await ensureFirstEncryptionIdentity(identity.client);

    expect(identity.userHasCrossSigningKeys).toHaveBeenCalledWith('@rainy:agent-room.test', true);
    expect(identity.bootstrapCrossSigning).toHaveBeenCalledOnce();
  });

  it('已有加密身份时绝不重建，新设备留给确认或恢复', async () => {
    const identity = cryptoWith(true);

    await ensureFirstEncryptionIdentity(identity.client);

    expect(identity.bootstrapCrossSigning).not.toHaveBeenCalled();
  });

  it('查询或上传失败时不影响登录', async () => {
    const identity = cryptoWith(false, new Error('offline'));

    await expect(ensureFirstEncryptionIdentity(identity.client)).resolves.toBeUndefined();
  });
});

function cryptoWith(hasKeys: boolean, bootstrapFailure?: Error) {
  const userHasCrossSigningKeys = vi.fn(() => Promise.resolve(hasKeys));
  const bootstrapCrossSigning = vi.fn(() =>
    bootstrapFailure === undefined ? Promise.resolve() : Promise.reject(bootstrapFailure),
  );
  const client = {
    getCrypto: () => ({ userHasCrossSigningKeys, bootstrapCrossSigning }),
    getUserId: () => '@rainy:agent-room.test',
  } as unknown as Pick<MatrixClient, 'getCrypto' | 'getUserId'>;
  return { client, userHasCrossSigningKeys, bootstrapCrossSigning };
}
