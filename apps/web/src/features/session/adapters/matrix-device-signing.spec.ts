import { describe, expect, it, vi } from 'vitest';

import { MatrixSecretStorageKeyCache } from '@/shared/matrix/matrix-secret-storage-key-cache';

import {
  type DeviceSigningClient,
  type EncryptionKeyEscrow,
  type EscrowedEncryptionKey,
  ensureDeviceSigned,
} from './matrix-device-signing';

const USER = '@rainy:agent-room.test';
const DEVICE = 'DESKTOP';
/** 新生成的钥匙交给服务器时的内容；交完以后内存里那份会被抹掉。 */
const GENERATED = new Uint8Array(32).fill(7);

describe('ensureDeviceSigned', () => {
  it('账户第一台设备：建立签名身份、建密钥存储和备份，把钥匙交给服务器', async () => {
    const account = fakeAccount({ identity: false });

    await expect(run(account)).resolves.toBe('established');

    expect(account.crypto.bootstrapCrossSigning).toHaveBeenCalledOnce();
    expect(account.crypto.bootstrapSecretStorage).toHaveBeenCalledWith(
      expect.objectContaining({ setupNewKeyBackup: true, setupNewSecretStorage: true }),
    );
    expect(account.stored).toEqual([{ keyId: 'NEW', key: GENERATED }]);
    expect(account.escrow.allowReset).not.toHaveBeenCalled();
  });

  it('手里有私钥、服务器的钥匙对得上：确认密钥存储里有私钥，加载备份钥匙', async () => {
    const account = fakeAccount({ escrowed: 'CURRENT', holdsKeys: true, signed: true });

    await expect(run(account)).resolves.toBe('ready');

    // 本机有私钥时 bootstrapCrossSigning({}) 只会把缺的私钥补存进密钥存储，不会重建。
    expect(account.crypto.bootstrapCrossSigning).toHaveBeenCalledWith({});
    expect(account.crypto.loadSessionBackupPrivateKeyFromSecretStorage).toHaveBeenCalledOnce();
    expect(account.crypto.crossSignDevice).not.toHaveBeenCalled();
    expect(account.crypto.bootstrapSecretStorage).not.toHaveBeenCalled();
    expect(account.stored).toEqual([]);
  });

  it('手里有私钥、服务器上没有钥匙：新建一把补交，沿用还在的备份', async () => {
    const account = fakeAccount({ backupKeyKnown: true, holdsKeys: true, signed: true });

    await expect(run(account)).resolves.toBe('ready');

    expect(account.crypto.bootstrapSecretStorage).toHaveBeenCalledWith(
      expect.objectContaining({ setupNewKeyBackup: false, setupNewSecretStorage: true }),
    );
    expect(account.stored).toEqual([{ keyId: 'NEW', key: GENERATED }]);
    expect(account.crypto.resetEncryption).not.toHaveBeenCalled();
  });

  it('手里有私钥、这台设备的签名没传上去：补签', async () => {
    const account = fakeAccount({ escrowed: 'CURRENT', holdsKeys: true });

    await expect(run(account)).resolves.toBe('ready');

    expect(account.crypto.crossSignDevice).toHaveBeenCalledWith(DEVICE);
    expect(account.crypto.resetEncryption).not.toHaveBeenCalled();
  });

  it('本机没有私钥、服务器的钥匙对得上：从密钥存储取回私钥，签好这台设备，在后台找回历史', async () => {
    const account = fakeAccount({ escrowed: 'CURRENT' });

    await expect(run(account)).resolves.toBe('signed');

    expect(account.unlock).toHaveBeenCalledWith('CURRENT', account.escrowedKey);
    expect(account.crypto.bootstrapCrossSigning).toHaveBeenCalledWith({});
    expect(account.crypto.crossSignDevice).toHaveBeenCalledWith(DEVICE);
    expect(account.crypto.restoreKeyBackup).toHaveBeenCalledOnce();
    expect(account.crypto.resetEncryption).not.toHaveBeenCalled();
    expect(account.stored).toEqual([]);
    expect(account.escrowedKey.every((byte) => byte === 0)).toBe(true);
  });

  it('本机没有私钥、服务器上没有能用的钥匙：请服务器开豁免，重建一次签名身份，交新钥匙', async () => {
    const cases = [
      { escrowed: null },
      { escrowed: 'OLD' },
      // 签过名也一样：没有私钥、服务器也没有钥匙，就没法签别的设备。
      { escrowed: null, signed: true },
      // 钥匙对得上，但密钥存储里没有签名私钥：bootstrapCrossSigning 会去重建，换成开了豁免的重建。
      { escrowed: 'CURRENT', keysInStorage: false },
    ] as const;
    for (const options of cases) {
      const account = fakeAccount(options);

      await expect(run(account)).resolves.toBe('reset');

      expect(account.escrow.allowReset).toHaveBeenCalledOnce();
      expect(account.crypto.resetEncryption).toHaveBeenCalledOnce();
      // 先开豁免，再重建。
      expect(account.escrow.allowReset.mock.invocationCallOrder[0]).toBeLessThan(
        account.crypto.resetEncryption.mock.invocationCallOrder[0] ?? 0,
      );
      expect(account.crypto.bootstrapCrossSigning).not.toHaveBeenCalled();
      expect(account.stored).toEqual([{ keyId: 'NEW', key: GENERATED }]);
      expect(account.unlock).not.toHaveBeenCalledWith('OLD', expect.anything());
    }
  });

  it('密钥存储里没有备份钥匙时照样签好，只是不找回历史', async () => {
    const account = fakeAccount({ escrowed: 'CURRENT' });
    account.crypto.loadSessionBackupPrivateKeyFromSecretStorage.mockRejectedValue(
      new Error('no backup key'),
    );

    await expect(run(account)).resolves.toBe('signed');

    expect(account.crypto.crossSignDevice).toHaveBeenCalledWith(DEVICE);
    expect(account.crypto.restoreKeyBackup).not.toHaveBeenCalled();
  });

  it('交钥匙失败时把错误交给调用方，生成的钥匙从内存里抹掉', async () => {
    const account = fakeAccount({ identity: false });
    account.escrow.store.mockRejectedValue(new Error('offline'));

    await expect(run(account)).rejects.toThrow('offline');

    expect(account.generatedKey.every((byte) => byte === 0)).toBe(true);
  });
});

function run(account: ReturnType<typeof fakeAccount>) {
  return ensureDeviceSigned(account.client, account.escrow, account.keys);
}

/**
 * 一个假的账户：`identity` 账户有没有签名身份，`holdsKeys` 本机有没有签名私钥，`signed` 这台设备
 * 签好没有，`escrowed` 服务器上那把钥匙的 ID（账户现在的密钥存储钥匙是 `CURRENT`），`keysInStorage`
 * 密钥存储里有没有签名私钥，`backupKeyKnown` 本机有没有备份钥匙。
 */
function fakeAccount({
  backupKeyKnown = false,
  escrowed = null,
  holdsKeys = false,
  identity = true,
  keysInStorage = true,
  signed = false,
}: {
  readonly backupKeyKnown?: boolean;
  readonly escrowed?: 'CURRENT' | 'OLD' | null;
  readonly holdsKeys?: boolean;
  readonly identity?: boolean;
  readonly keysInStorage?: boolean;
  readonly signed?: boolean;
}) {
  let defaultKey: string | null = identity ? 'CURRENT' : null;
  const generatedKey = new Uint8Array(32).fill(7);
  const escrowedKey = new Uint8Array(32).fill(9);
  const stored: EscrowedEncryptionKey[] = [];
  const crypto = {
    bootstrapCrossSigning: vi.fn(() => Promise.resolve()),
    bootstrapSecretStorage: vi.fn(
      async (opts: { readonly createSecretStorageKey?: () => Promise<unknown> }) => {
        await opts.createSecretStorageKey?.();
        defaultKey = 'NEW';
      },
    ),
    checkKeyBackupAndEnable: vi.fn(() =>
      Promise.resolve({ trustInfo: { matchesDecryptionKey: backupKeyKnown, trusted: true } }),
    ),
    createRecoveryKeyFromPassphrase: vi.fn(() =>
      Promise.resolve({ keyInfo: {}, privateKey: generatedKey }),
    ),
    crossSignDevice: vi.fn(() => Promise.resolve()),
    getCrossSigningStatus: vi.fn(() =>
      Promise.resolve({
        privateKeysCachedLocally: {
          masterKey: holdsKeys,
          selfSigningKey: holdsKeys,
          userSigningKey: holdsKeys,
        },
        privateKeysInSecretStorage: identity && keysInStorage,
        publicKeysOnDevice: identity,
      }),
    ),
    getDeviceVerificationStatus: vi.fn(() => Promise.resolve({ signedByOwner: signed })),
    loadSessionBackupPrivateKeyFromSecretStorage: vi.fn(() => Promise.resolve()),
    resetEncryption: vi.fn(() => Promise.resolve()),
    restoreKeyBackup: vi.fn(() => Promise.resolve({ imported: 0, total: 0 })),
    userHasCrossSigningKeys: vi.fn(() => Promise.resolve(identity)),
  };
  const client = {
    getCrypto: () => crypto,
    getDeviceId: () => DEVICE,
    getUserId: () => USER,
    secretStorage: {
      checkKey: vi.fn((key: Uint8Array) =>
        Promise.resolve(key.every((byte, index) => byte === escrowedKey[index])),
      ),
      getKey: vi.fn(() => Promise.resolve(defaultKey === null ? null : [defaultKey, {}])),
    },
  } as unknown as DeviceSigningClient;
  const escrow = {
    allowReset: vi.fn(() => Promise.resolve()),
    fetch: vi.fn(() =>
      Promise.resolve(escrowed === null ? null : { key: escrowedKey, keyId: escrowed }),
    ),
    store: vi.fn((key: EscrowedEncryptionKey) => {
      stored.push({ key: Uint8Array.from(key.key), keyId: key.keyId });
      return Promise.resolve();
    }),
  } satisfies EncryptionKeyEscrow;
  const keys = new MatrixSecretStorageKeyCache();
  const unlock = vi.spyOn(keys, 'unlock');
  return { client, crypto, escrow, escrowedKey, generatedKey, keys, stored, unlock };
}
