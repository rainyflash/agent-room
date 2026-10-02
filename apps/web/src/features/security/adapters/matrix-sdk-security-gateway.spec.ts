import type { MatrixClient } from 'matrix-js-sdk';
import type { CryptoApi, DeviceVerificationStatus } from 'matrix-js-sdk/lib/crypto-api/index.js';
import { describe, expect, it, vi } from 'vitest';

import { MatrixSdkSecurityGateway } from '@/features/security/adapters/matrix-sdk-security-gateway';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

const USER = '@alice:agent-room.test';

type FakeDevice = {
  readonly deviceId: string;
  readonly displayName?: string;
  readonly fingerprint?: string;
  readonly status: Partial<DeviceVerificationStatus> | null;
};

describe('MatrixSdkSecurityGateway', () => {
  it('没有活跃 Matrix 客户端或没有加密时返回明确失败', async () => {
    await expect(gatewayFor(null).inspect()).resolves.toEqual({
      error: { code: 'security.matrix_unavailable', retryable: true },
      ok: false,
    });
    await expect(gatewayFor(client(undefined)).inspect()).resolves.toEqual({
      error: { code: 'security.crypto_unavailable', retryable: true },
      ok: false,
    });
  });

  it('列出你的设备：这台排在最前，只看由你的签名身份签过没有', async () => {
    const crypto = cryptoWith([
      { deviceId: 'OLD-LAPTOP', status: { crossSigningVerified: false, signedByOwner: false } },
      {
        deviceId: 'DESKTOP',
        displayName: 'Agent Room Desktop',
        fingerprint: 'ed25519-desktop',
        status: { crossSigningVerified: false, signedByOwner: true },
      },
      {
        deviceId: 'WEB',
        displayName: 'Agent Room Web',
        // 本机对自己设备的“本地可信”不算签名。
        status: { crossSigningVerified: false, localVerified: true, signedByOwner: false },
      },
      { deviceId: 'GONE', status: null },
      { deviceId: 'PHONE', status: { crossSigningVerified: true, signedByOwner: true } },
    ]);

    const result = await gatewayFor(client(crypto, 'WEB')).inspect();

    expect(result).toEqual({
      ok: true,
      value: {
        currentDeviceId: 'WEB',
        devices: [
          {
            current: true,
            deviceId: 'WEB',
            displayName: 'Agent Room Web',
            trust: 'unverified',
            userId: USER,
          },
          {
            current: false,
            deviceId: 'DESKTOP',
            displayName: 'Agent Room Desktop',
            fingerprint: 'ed25519-desktop',
            trust: 'signed',
            userId: USER,
          },
          { current: false, deviceId: 'GONE', trust: 'unknown', userId: USER },
          { current: false, deviceId: 'OLD-LAPTOP', trust: 'unverified', userId: USER },
          { current: false, deviceId: 'PHONE', trust: 'verified', userId: USER },
        ],
        userId: USER,
      },
    });
  });

  it('读设备出错时说查不到', async () => {
    const crypto = {
      getUserDeviceInfo: () => Promise.reject(new Error('offline')),
    } as unknown as CryptoApi;

    await expect(gatewayFor(client(crypto)).inspect()).resolves.toEqual({
      error: { code: 'security.inspection_failed', retryable: true },
      ok: false,
    });
  });

  it('客户端换了或有新动静（比如刚签好）时通知界面重读', () => {
    const listeners = new Set<() => void>();
    const source: MatrixClientSource = {
      current: () => null,
      subscribe: (listener) => {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
    };
    const gateway = new MatrixSdkSecurityGateway(source);
    const changed = vi.fn();
    const unsubscribe = gateway.subscribe(changed);

    for (const listener of listeners) listener();
    expect(changed).toHaveBeenCalledOnce();

    unsubscribe();
    for (const listener of listeners) listener();
    expect(changed).toHaveBeenCalledOnce();
  });
});

function gatewayFor(current: MatrixClient | null): MatrixSdkSecurityGateway {
  return new MatrixSdkSecurityGateway({ current: () => current, subscribe: () => () => undefined });
}

function client(crypto: CryptoApi | undefined, deviceId = 'WEB'): MatrixClient {
  return {
    getCrypto: () => crypto,
    getDeviceId: () => deviceId,
    getUserId: () => USER,
  } as unknown as MatrixClient;
}

function cryptoWith(devices: readonly FakeDevice[]): CryptoApi {
  return {
    getDeviceVerificationStatus: (_userId: string, deviceId: string) =>
      Promise.resolve(devices.find((device) => device.deviceId === deviceId)?.status ?? null),
    getUserDeviceInfo: (userIds: string[]) => {
      expect(userIds).toEqual([USER]);
      return Promise.resolve(
        new Map([
          [
            USER,
            new Map(
              devices.map((device) => [
                device.deviceId,
                {
                  deviceId: device.deviceId,
                  displayName: device.displayName,
                  getFingerprint: () => device.fingerprint,
                },
              ]),
            ),
          ],
        ]),
      );
    },
  } as unknown as CryptoApi;
}
