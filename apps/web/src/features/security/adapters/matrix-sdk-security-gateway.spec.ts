import type { MatrixClient } from 'matrix-js-sdk';
import type { CryptoApi } from 'matrix-js-sdk/lib/crypto-api/index.js';
import { describe, expect, it } from 'vitest';

import { MatrixSdkSecurityGateway } from '@/features/security/adapters/matrix-sdk-security-gateway';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

describe('MatrixSdkSecurityGateway', () => {
  it('没有活跃 Matrix 客户端时返回明确失败', async () => {
    const gateway = new MatrixSdkSecurityGateway(source(null));

    await expect(gateway.inspect()).resolves.toEqual({
      error: { code: 'security.matrix_unavailable', retryable: true },
      ok: false,
    });
  });

  it('投影官方 Crypto 状态并拒绝未加密房间', async () => {
    const crypto = {
      checkKeyBackupAndEnable: () =>
        Promise.resolve({
          backupInfo: {},
          trustInfo: { matchesDecryptionKey: true, trusted: true },
        }),
      getDeviceVerificationStatus: () =>
        Promise.resolve({
          isVerified: () => true,
          signedByOwner: true,
        }),
      getSecretStorageStatus: () => Promise.resolve({ ready: true }),
      getUserDeviceInfo: () =>
        Promise.resolve(
          new Map([
            [
              '@alice:agent-room.test',
              new Map([
                [
                  'ALICE-WEB',
                  {
                    deviceId: 'ALICE-WEB',
                    displayName: 'Agent Room Web',
                    getFingerprint: () => 'ed25519-fingerprint',
                  },
                ],
              ]),
            ],
          ]),
        ),
      getVersion: () => 'Rust SDK test',
      isCrossSigningReady: () => Promise.resolve(true),
      isEncryptionEnabledInRoom: () => Promise.resolve(false),
    } as unknown as CryptoApi;
    const client = {
      getCrypto: () => crypto,
      getDeviceId: () => 'ALICE-WEB',
      getRoom: () => null,
      getUserId: () => '@alice:agent-room.test',
    } as unknown as MatrixClient;
    const gateway = new MatrixSdkSecurityGateway(source(client));

    const result = await gateway.inspect({ roomId: '!private:agent-room.test' });

    expect(result.ok).toBe(true);
    if (!result.ok) {
      return;
    }
    expect(result.value).toMatchObject({
      backup: 'ready',
      blockers: ['room_unencrypted'],
      crossSigningReady: true,
      currentDeviceId: 'ALICE-WEB',
      kind: 'blocked',
      roomEncryption: 'unencrypted',
      secretStorageReady: true,
      sendAllowed: false,
    });
    expect(result.value.devices).toEqual([
      {
        current: true,
        deviceId: 'ALICE-WEB',
        displayName: 'Agent Room Web',
        fingerprint: 'ed25519-fingerprint',
        trust: 'verified',
        userId: '@alice:agent-room.test',
      },
    ]);
  });
});

function source(client: MatrixClient | null): MatrixClientSource {
  return {
    current: () => client,
    subscribe: () => () => undefined,
  };
}
