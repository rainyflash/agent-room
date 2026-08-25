import type { MatrixClient } from 'matrix-js-sdk';
import {
  ImportRoomKeyStage,
  VerificationPhase,
  type CryptoApi,
  type VerificationRequest,
} from 'matrix-js-sdk/lib/crypto-api/index.js';
import { describe, expect, it, vi } from 'vitest';

import { MatrixSdkSecurityGateway } from '@/features/security/adapters/matrix-sdk-security-gateway';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import { MatrixSecretStorageKeyCache } from '@/shared/matrix/matrix-secret-storage-key-cache';

describe('MatrixSdkSecurityGateway', () => {
  it('没有活跃 Matrix 客户端时返回明确失败', async () => {
    const gateway = gatewayFor(null);

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
    const gateway = gatewayFor(client);

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

  it('当前设备与指定设备都通过官方交互式验证事务启动', async () => {
    const requestOwnUserVerification = vi.fn(() => Promise.resolve(pendingVerificationRequest()));
    const requestDeviceVerification = vi.fn(() => Promise.resolve(pendingVerificationRequest()));
    const crypto = {
      requestDeviceVerification,
      requestOwnUserVerification,
    } as unknown as CryptoApi;
    const gateway = gatewayFor(cryptoClient(crypto));

    const current = await gateway.beginVerification();
    expect(current.ok).toBe(true);
    if (current.ok) {
      current.value.dispose();
    }
    const other = await gateway.beginVerification({ targetDeviceId: 'ALICE-LAPTOP' });
    expect(other.ok).toBe(true);
    if (other.ok) {
      other.value.dispose();
    }

    expect(requestOwnUserVerification).toHaveBeenCalledOnce();
    expect(requestDeviceVerification).toHaveBeenCalledWith(
      '@alice:agent-room.test',
      'ALICE-LAPTOP',
    );
  });

  it('建立恢复链后只返回一次性恢复密钥并覆写生成态私钥', async () => {
    const privateKey = Uint8Array.from({ length: 32 }, (_, index) => index + 1);
    const bootstrapInputs: unknown[] = [];
    const crypto = {
      bootstrapSecretStorage: (options: unknown) => {
        bootstrapInputs.push(options);
        return Promise.resolve();
      },
      createRecoveryKeyFromPassphrase: () =>
        Promise.resolve({
          encodedPrivateKey: 'EsTc r7Cy 4abc recovery key',
          privateKey,
        }),
      getSecretStorageStatus: () =>
        Promise.resolve({ defaultKeyId: null, ready: false, secretStorageKeyValidityMap: {} }),
      isCrossSigningReady: () => Promise.resolve(true),
    } as unknown as CryptoApi;
    const client = cryptoClient(crypto);

    const result = await gatewayFor(client).setupRecovery({
      passphrase: 'correct horse battery staple',
    });

    expect(result).toEqual({
      ok: true,
      value: { recoveryKey: 'EsTc r7Cy 4abc recovery key' },
    });
    expect(bootstrapInputs).toHaveLength(1);
    expect([...privateKey]).toEqual(Array.from({ length: 32 }, () => 0));
  });

  it('用 Secret Storage 恢复交叉签名、签发当前设备并导入历史密钥', async () => {
    const actions: string[] = [];
    const progress: unknown[] = [];
    const crypto = {
      bootstrapCrossSigning: () => {
        actions.push('cross-signing');
        return Promise.resolve();
      },
      crossSignDevice: (deviceId: string) => {
        actions.push(`sign:${deviceId}`);
        return Promise.resolve();
      },
      getDeviceVerificationStatus: () =>
        Promise.resolve({ isVerified: () => false, signedByOwner: false }),
      loadSessionBackupPrivateKeyFromSecretStorage: () => {
        actions.push('load-backup-key');
        return Promise.resolve();
      },
      restoreKeyBackup: (options: {
        readonly progressCallback?: (value: {
          readonly failures?: number;
          readonly stage: ImportRoomKeyStage;
          readonly successes?: number;
          readonly total?: number;
        }) => void;
      }) => {
        actions.push('restore');
        options.progressCallback?.({ stage: ImportRoomKeyStage.Fetch });
        options.progressCallback?.({
          failures: 1,
          stage: ImportRoomKeyStage.LoadKeys,
          successes: 7,
          total: 8,
        });
        return Promise.resolve({ imported: 7, total: 8 });
      },
    } as unknown as CryptoApi;
    const client = {
      getCrypto: () => crypto,
      getDeviceId: () => 'ALICE-WEB',
      getUserId: () => '@alice:agent-room.test',
      secretStorage: {
        checkKey: () => Promise.resolve(true),
        getKey: () =>
          Promise.resolve([
            'recovery-key-id',
            {
              algorithm: 'm.secret_storage.v1.aes-hmac-sha2',
              iv: 'iv',
              mac: 'mac',
              passphrase: {
                algorithm: 'm.pbkdf2',
                iterations: 10,
                salt: 'fixed-test-salt',
              },
            },
          ]),
      },
    } as unknown as MatrixClient;
    const cache = new MatrixSecretStorageKeyCache();
    const gateway = new MatrixSdkSecurityGateway(source(client), cache);

    const result = await gateway.recover({ credential: 'correct horse battery staple' }, (value) =>
      progress.push(value),
    );

    expect(result).toEqual({ ok: true, value: { imported: 7, total: 8 } });
    expect(actions).toEqual(['cross-signing', 'load-backup-key', 'sign:ALICE-WEB', 'restore']);
    expect(progress).toEqual([
      { stage: 'fetching' },
      { failures: 1, imported: 7, stage: 'importing', total: 8 },
    ]);
    await expect(
      cache.callbacks.getSecretStorageKey?.(
        { keys: { 'recovery-key-id': {} as never } },
        'm.cross_signing.master',
      ),
    ).resolves.toEqual(['recovery-key-id', expect.any(Uint8Array)]);
  });
});

function gatewayFor(client: MatrixClient | null): MatrixSdkSecurityGateway {
  return new MatrixSdkSecurityGateway(source(client), new MatrixSecretStorageKeyCache());
}

function source(client: MatrixClient | null): MatrixClientSource {
  return {
    current: () => client,
    subscribe: () => () => undefined,
  };
}

function cryptoClient(crypto: CryptoApi): MatrixClient {
  return {
    getCrypto: () => crypto,
    getDeviceId: () => 'ALICE-WEB',
    getUserId: () => '@alice:agent-room.test',
  } as unknown as MatrixClient;
}

function pendingVerificationRequest(): VerificationRequest {
  const request = {
    cancellationCode: null,
    cancel: () => Promise.resolve(),
    off: () => request,
    on: () => request,
    phase: VerificationPhase.Requested,
    startVerification: () => Promise.reject(new Error('尚未被另一台设备接受。')),
    transactionId: 'verification-transaction',
    verifier: undefined,
  };
  return request as unknown as VerificationRequest;
}
