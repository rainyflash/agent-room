import type { MatrixClient } from 'matrix-js-sdk';
import type { CrossSigningStatus, CryptoApi } from 'matrix-js-sdk/lib/crypto-api/index.js';

import type { MatrixSecretStorageKeyCache } from '@/shared/matrix/matrix-secret-storage-key-cache';

/** 服务器替账户保管的密钥存储钥匙（ADR 0011）：以前让人保管的“恢复密钥”。 */
export type EscrowedEncryptionKey = {
  readonly keyId: string;
  readonly key: Uint8Array;
};

/** 控制面的 `/account/encryption-key` 与 `/account/encryption-reset`。请求失败时抛错。 */
export type EncryptionKeyEscrow = {
  /** 服务器上还没有就是 `null`。 */
  fetch(): Promise<EscrowedEncryptionKey | null>;
  store(key: EscrowedEncryptionKey): Promise<void>;
  /**
   * 重建签名身份时，请控制面以应用服务的身份替本人上传新的签名公钥（Matrix
   * `keys/device_signing/upload` 的正文，不带认证）。被拒绝时抛 {@link CrossSigningUploadFailure}。
   */
  replaceCrossSigningKeys(keys: Readonly<Record<string, unknown>>): Promise<void>;
};

/** 控制面没能代传新的签名公钥。`retryable`：暂时不可用，可以再试；否则是被拒绝了。 */
export class CrossSigningUploadFailure extends Error {
  constructor(
    readonly retryable: boolean,
    message: string,
  ) {
    super(message);
    this.name = 'CrossSigningUploadFailure';
  }
}

/**
 * 换签名身份要交互认证；没接 MAS 的 Synapse 只认应用服务的免认证上传。matrix-js-sdk 带着这个
 * 认证标记去上传时，`cross-signing-upload-route.ts` 把这一个请求改送控制面代传。
 */
export const CONTROL_PLANE_UPLOAD = 'io.github.rainyflash.agentroom.control_plane_upload';

/**
 * 这一遍做了什么：
 * - `established`：账户第一台设备，建立了签名身份并把钥匙交给服务器；
 * - `ready`：这台设备本来就签好了（需要的话补交了钥匙）；
 * - `signed`：用服务器上的钥匙签好了这台设备，并在后台找回历史；
 * - `reset`：找不到能用的钥匙，重建了一次签名身份。
 */
export type DeviceSigningOutcome = 'established' | 'ready' | 'signed' | 'reset';

export type DeviceSigningClient = Pick<
  MatrixClient,
  'getCrypto' | 'getDeviceId' | 'getUserId' | 'secretStorage'
>;

/**
 * 登录后让这台设备由账户的签名身份签好（`specs/device-signing/design.md`）。每次启动都可以跑，
 * 每一步都能重复做：
 * 1. 账户还没有签名身份：建立身份、建密钥存储和密钥备份，把钥匙交给服务器；
 * 2. 本机拿着签名私钥：没签好就签上；服务器上没有对得上的钥匙就新建一把补交；
 * 3. 否则用服务器上的钥匙打开密钥存储、取回签名私钥、签好这台设备、找回历史；没有能用的钥匙
 *    （或者密钥存储里没有签名私钥）就重建一次签名身份。
 */
export async function ensureDeviceSigned(
  client: DeviceSigningClient,
  escrow: EncryptionKeyEscrow,
  keys: MatrixSecretStorageKeyCache,
): Promise<DeviceSigningOutcome> {
  const crypto = client.getCrypto();
  const userId = client.getUserId();
  const deviceId = client.getDeviceId();
  if (crypto === undefined || userId === null || deviceId === null) {
    throw new Error('Matrix 加密还没有初始化好。');
  }
  // 自己的身份每次都重新查一遍：别的设备重建过签名身份时，本机对不上的旧私钥随之清掉。
  if (!(await crypto.userHasCrossSigningKeys(userId, true))) {
    await crypto.bootstrapCrossSigning({
      authUploadDeviceSigningKeys: uploadWithoutAuthentication,
    });
    await createSecretStorage(client, crypto, escrow, true);
    return 'established';
  }
  const escrowed = await escrow.fetch();
  try {
    const usable = escrowed !== null && (await unlocksAccount(client, escrowed));
    if (usable) keys.unlock(escrowed.keyId, escrowed.key);
    const status = await crypto.getCrossSigningStatus();
    if (holdsSigningKeys(status)) {
      await keepSigned(client, crypto, escrow, usable);
      return 'ready';
    }
    if (usable && status.privateKeysInSecretStorage) {
      // 本机没有签名私钥时，这一步从密钥存储取回私钥并签好这台设备。
      await crypto.bootstrapCrossSigning({});
      await crypto.crossSignDevice(deviceId);
      if (await loadBackupKey(crypto)) {
        void crypto.restoreKeyBackup().catch(ignoreBackgroundFailure);
      }
      return 'signed';
    }
  } finally {
    escrowed?.key.fill(0);
  }
  // 新的签名身份（签好这台设备）、新的密钥备份；旧的密钥存储和备份随之作废。新签名公钥由控制面代传。
  await crypto.resetEncryption(uploadThroughControlPlane);
  await createSecretStorage(client, crypto, escrow, false);
  return 'reset';
}

/** 账户第一次建立签名身份时，上传不需要交互认证。 */
async function uploadWithoutAuthentication(
  makeRequest: (authData: null) => Promise<unknown>,
): Promise<void> {
  await makeRequest(null);
}

/** 换签名身份：带上我们的认证标记，上传改由控制面以应用服务的身份代传。 */
async function uploadThroughControlPlane(
  makeRequest: (authData: { readonly type: string }) => Promise<unknown>,
): Promise<void> {
  await makeRequest({ type: CONTROL_PLANE_UPLOAD });
}

/** 本机缓存着三把签名私钥（对不上账户现在签名身份的，在查身份时已经清掉了）。 */
function holdsSigningKeys(status: CrossSigningStatus): boolean {
  const cached = status.privateKeysCachedLocally;
  return cached.masterKey && cached.selfSigningKey && cached.userSigningKey;
}

/**
 * 本机拿着签名私钥：这台设备没签好就签上（比如上次的签名没传上去）。服务器上的钥匙对得上就
 * 确认密钥存储里有签名私钥（缺了只补存，不会重建）并加载备份钥匙；对不上就新建一把补交。
 */
async function keepSigned(
  client: DeviceSigningClient,
  crypto: CryptoApi,
  escrow: EncryptionKeyEscrow,
  usable: boolean,
): Promise<void> {
  const userId = client.getUserId() ?? '';
  const deviceId = client.getDeviceId() ?? '';
  const verification = await crypto.getDeviceVerificationStatus(userId, deviceId);
  if (verification?.signedByOwner !== true) await crypto.crossSignDevice(deviceId);
  if (usable) {
    await crypto.bootstrapCrossSigning({});
    await loadBackupKey(crypto);
  } else {
    await createSecretStorage(client, crypto, escrow, await needsNewBackup(crypto));
  }
}

/** 服务器上的钥匙能不能打开账户现在的密钥存储。 */
async function unlocksAccount(
  client: DeviceSigningClient,
  escrowed: EscrowedEncryptionKey,
): Promise<boolean> {
  const current = await client.secretStorage.getKey();
  if (current === null) return false;
  const [keyId, keyInfo] = current;
  if (keyId !== escrowed.keyId) return false;
  return await client.secretStorage.checkKey(Uint8Array.from(escrowed.key), keyInfo);
}

/** 密钥备份还在、本机也有它的钥匙就沿用，否则新建一份。 */
async function needsNewBackup(crypto: CryptoApi): Promise<boolean> {
  const backup = await crypto.checkKeyBackupAndEnable();
  return backup?.trustInfo.matchesDecryptionKey !== true;
}

/**
 * 从密钥存储加载密钥备份的钥匙，之后新收到的房间密钥都会进备份。密钥存储里没有备份钥匙（比如
 * 以前只建了签名身份）时加载不了：这台设备照样签好了，只是找不回历史，所以不算失败。
 */
async function loadBackupKey(crypto: CryptoApi): Promise<boolean> {
  try {
    await crypto.loadSessionBackupPrivateKeyFromSecretStorage();
    await crypto.checkKeyBackupAndEnable();
    return true;
  } catch {
    return false;
  }
}

/**
 * 新建一把钥匙和密钥存储（签名私钥和备份钥匙都存进去），再把钥匙交给服务器。没交成也不要紧：
 * 这台设备手里有签名私钥，下次启动按第 2 步再补交。
 */
async function createSecretStorage(
  client: DeviceSigningClient,
  crypto: CryptoApi,
  escrow: EncryptionKeyEscrow,
  newBackup: boolean,
): Promise<void> {
  const generated = await crypto.createRecoveryKeyFromPassphrase();
  try {
    await crypto.bootstrapSecretStorage({
      createSecretStorageKey: () => Promise.resolve(generated),
      setupNewKeyBackup: newBackup,
      setupNewSecretStorage: true,
    });
    const current = await client.secretStorage.getKey();
    if (current === null) throw new Error('密钥存储没有建好。');
    await escrow.store({ key: generated.privateKey, keyId: current[0] });
  } finally {
    generated.privateKey.fill(0);
  }
}

function ignoreBackgroundFailure(error: unknown): void {
  void error;
}
