import {
  CryptoEvent,
  VerificationPhase,
  VerificationRequestEvent,
  type CryptoApi,
  type DeviceVerificationStatus,
  type VerificationRequest,
} from 'matrix-js-sdk/lib/crypto-api/index.js';
import type { Device, MatrixClient } from 'matrix-js-sdk';

import { MatrixSdkVerificationSession } from '@/features/security/adapters/matrix-sdk-verification-session';
import {
  evaluateMatrixSecurity,
  isValidRecoveryPassphrase,
  type MatrixBackupState,
  type MatrixDeviceTrust,
  type MatrixIncomingVerification,
  type MatrixRecoveryProgress,
  type MatrixRecoveryRequest,
  type MatrixRecoveryResult,
  type MatrixRecoverySetupRequest,
  type MatrixRecoverySetupResult,
  type MatrixSecurityDevice,
  type MatrixSecurityEvidence,
  type MatrixSecurityFailure,
  type MatrixSecurityGateway,
  type MatrixSecurityInspection,
  type MatrixSecuritySnapshot,
  type MatrixVerificationRequest,
  type MatrixVerificationSession,
} from '@/features/security/domain/matrix-security';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import { MatrixSecretStorageKeyCache } from '@/shared/matrix/matrix-secret-storage-key-cache';
import { err, ok, type Result } from '@/shared/result';

export class MatrixSdkSecurityGateway implements MatrixSecurityGateway {
  readonly #clients: MatrixClientSource;
  readonly #incoming = new Map<string, PendingVerification>();
  readonly #listeners = new Set<() => void>();
  readonly #secretStorageKeys: MatrixSecretStorageKeyCache;
  #activeClient: MatrixClient | null = null;

  constructor(clients: MatrixClientSource, secretStorageKeys: MatrixSecretStorageKeyCache) {
    this.#clients = clients;
    this.#secretStorageKeys = secretStorageKeys;
    this.#synchronizeClient();
    this.#clients.subscribe(this.#synchronizeClient);
  }

  async acceptIncomingVerification(
    requestId: string,
  ): Promise<Result<MatrixVerificationSession, MatrixSecurityFailure>> {
    const pending = this.#incoming.get(requestId);
    if (pending?.request.phase !== VerificationPhase.Requested) {
      return err(failure('security.verification_unavailable', false));
    }
    try {
      await pending.request.accept();
      this.#removeIncoming(requestId);
      const session = new MatrixSdkVerificationSession(
        pending.request,
        pending.notice.sourceDeviceId,
      );
      return ok(session);
    } catch {
      return err(failure('security.verification_failed', true));
    }
  }

  async beginVerification(
    request: MatrixVerificationRequest = {},
  ): Promise<Result<MatrixVerificationSession, MatrixSecurityFailure>> {
    const active = activeCryptoClient(this.#clients.current());
    if (!active.ok) {
      return active;
    }
    const targetDeviceId = request.targetDeviceId;
    if (targetDeviceId !== undefined && !isValidDeviceId(targetDeviceId)) {
      return err(failure('security.verification_unavailable', false));
    }

    try {
      const verificationRequest =
        targetDeviceId === undefined || targetDeviceId === active.value.deviceId
          ? await active.value.crypto.requestOwnUserVerification()
          : await active.value.crypto.requestDeviceVerification(
              active.value.userId,
              targetDeviceId,
            );
      // to-device 验证由接受请求的一侧选择 SAS 方法；请求侧只等待 start 事件。
      const session = new MatrixSdkVerificationSession(verificationRequest, targetDeviceId, false);
      return ok(session);
    } catch {
      return err(failure('security.verification_failed', true));
    }
  }

  async declineIncomingVerification(
    requestId: string,
  ): Promise<Result<void, MatrixSecurityFailure>> {
    const pending = this.#incoming.get(requestId);
    if (pending === undefined) {
      return err(failure('security.verification_unavailable', false));
    }
    try {
      await pending.request.cancel();
      this.#removeIncoming(requestId);
      return ok(undefined);
    } catch {
      return err(failure('security.verification_failed', true));
    }
  }

  async establishIdentity(): Promise<Result<void, MatrixSecurityFailure>> {
    const active = activeCryptoClient(this.#clients.current());
    if (!active.ok) {
      return active;
    }
    try {
      await active.value.crypto.bootstrapCrossSigning({
        authUploadDeviceSigningKeys: async (makeRequest) => {
          await makeRequest(null);
        },
      });
      this.#notify();
      return ok(undefined);
    } catch {
      return err(failure('security.identity_bootstrap_failed', true));
    }
  }

  getIncomingVerification(): MatrixIncomingVerification | null {
    return this.#incoming.values().next().value?.notice ?? null;
  }

  async inspect(
    inspection: MatrixSecurityInspection = {},
  ): Promise<Result<MatrixSecuritySnapshot, MatrixSecurityFailure>> {
    const client = this.#clients.current();
    if (client === null) {
      return err(failure('security.matrix_unavailable', true));
    }
    const crypto = client.getCrypto();
    if (crypto === undefined) {
      return err(failure('security.crypto_unavailable', true));
    }
    const userId = client.getUserId();
    const deviceId = client.getDeviceId();
    if (userId === null || deviceId === null) {
      return err(failure('security.identity_unavailable', false));
    }

    try {
      const userIds = participantUserIds(client, inspection.roomId, userId);
      const [
        crossSigningReady,
        crossSigningStatus,
        secretStorage,
        backup,
        roomEncryption,
        deviceMap,
      ] = await Promise.all([
        crypto.isCrossSigningReady(),
        crypto.getCrossSigningStatus(),
        crypto.getSecretStorageStatus(),
        inspectBackup(crypto),
        inspectRoomEncryption(crypto, inspection.roomId),
        crypto.getUserDeviceInfo(userIds, true),
      ]);
      const devices = await inspectDevices(crypto, deviceMap, userId, deviceId);
      const evidence: MatrixSecurityEvidence = Object.freeze({
        backup,
        crossSigningIdentityExists: crossSigningStatus.publicKeysOnDevice,
        crossSigningReady,
        cryptoVersion: crypto.getVersion(),
        currentDeviceId: deviceId,
        devices,
        roomEncryption,
        ...(inspection.roomId === undefined ? {} : { roomId: inspection.roomId }),
        secretStorageReady: secretStorage.ready,
        userId,
      });

      return ok(Object.freeze({ ...evidence, ...evaluateMatrixSecurity(evidence) }));
    } catch {
      return err(failure('security.inspection_failed', true));
    }
  }

  async setupRecovery(
    request: MatrixRecoverySetupRequest,
  ): Promise<Result<MatrixRecoverySetupResult, MatrixSecurityFailure>> {
    if (!isValidRecoveryPassphrase(request.passphrase)) {
      return err(failure('security.recovery_credential_invalid', false));
    }
    const active = activeCryptoClient(this.#clients.current());
    if (!active.ok) {
      return active;
    }

    try {
      if (!(await active.value.crypto.isCrossSigningReady())) {
        return err(failure('security.verification_required', false));
      }
      const secretStorageStatus = await active.value.crypto.getSecretStorageStatus();
      if (secretStorageStatus.defaultKeyId !== null) {
        return err(failure('security.recovery_already_configured', false));
      }
      const generated = await active.value.crypto.createRecoveryKeyFromPassphrase(
        request.passphrase,
      );
      try {
        await active.value.crypto.bootstrapSecretStorage({
          createSecretStorageKey: () => Promise.resolve(generated),
          setupNewKeyBackup: true,
        });
        return generated.encodedPrivateKey === undefined
          ? err(failure('security.recovery_setup_failed', true))
          : ok(Object.freeze({ recoveryKey: generated.encodedPrivateKey }));
      } finally {
        generated.privateKey.fill(0);
      }
    } catch {
      return err(failure('security.recovery_setup_failed', true));
    }
  }

  async recover(
    request: MatrixRecoveryRequest,
    onProgress: (progress: MatrixRecoveryProgress) => void = ignoreRecoveryProgress,
  ): Promise<Result<MatrixRecoveryResult, MatrixSecurityFailure>> {
    if (request.credential.length === 0 || request.credential.length > 1_024) {
      return err(failure('security.recovery_credential_invalid', false));
    }
    const active = activeCryptoClient(this.#clients.current());
    if (!active.ok) {
      return active;
    }

    const storedKey = await active.value.client.secretStorage.getKey().catch(() => null);
    if (storedKey === null) {
      return err(failure('security.recovery_key_missing', false));
    }
    const [keyId, keyInfo] = storedKey;
    let key: Uint8Array<ArrayBuffer> | null = null;
    try {
      key = await decodeRecoveryCredential(request.credential, keyInfo.passphrase);
      if (!(await active.value.client.secretStorage.checkKey(key, keyInfo))) {
        key.fill(0);
        return err(failure('security.recovery_key_rejected', false));
      }
    } catch {
      key?.fill(0);
      return err(failure('security.recovery_key_rejected', false));
    }

    this.#secretStorageKeys.unlock(keyId, key);
    key.fill(0);
    try {
      const { crypto, deviceId, userId } = active.value;
      await crypto.bootstrapCrossSigning({});
      await crypto.loadSessionBackupPrivateKeyFromSecretStorage();
      const verification = await crypto.getDeviceVerificationStatus(userId, deviceId);
      if (verification?.isVerified() !== true) {
        await crypto.crossSignDevice(deviceId);
      }
      const restored = await crypto.restoreKeyBackup({
        progressCallback: (progress) => {
          onProgress(
            !('successes' in progress)
              ? { stage: 'fetching' }
              : {
                  failures: progress.failures,
                  imported: progress.successes,
                  stage: 'importing',
                  total: progress.total,
                },
          );
        },
      });
      return ok(Object.freeze({ imported: restored.imported, total: restored.total }));
    } catch {
      return err(failure('security.recovery_failed', true));
    }
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  readonly #handleIncomingVerification = (request: VerificationRequest): void => {
    const userId = this.#activeClient?.getUserId();
    if (
      userId === null ||
      userId === undefined ||
      request.otherUserId !== userId ||
      !request.isSelfVerification ||
      request.phase !== VerificationPhase.Requested
    ) {
      return;
    }
    const requestId = incomingRequestId(request);
    if (this.#incoming.has(requestId)) {
      return;
    }
    const notice = Object.freeze({
      requestId,
      ...(request.otherDeviceId === undefined ? {} : { sourceDeviceId: request.otherDeviceId }),
      sourceUserId: request.otherUserId,
    });
    const onChange = (): void => {
      if (!request.pending) {
        this.#removeIncoming(requestId);
      }
    };
    request.on(VerificationRequestEvent.Change, onChange);
    this.#incoming.set(requestId, { notice, onChange, request });
    this.#notify();
  };

  readonly #synchronizeClient = (): void => {
    const next = this.#clients.current();
    if (next !== this.#activeClient) {
      this.#activeClient?.off(
        CryptoEvent.VerificationRequestReceived,
        this.#handleIncomingVerification,
      );
      this.#clearIncoming();
      this.#activeClient = next;
      next?.on(CryptoEvent.VerificationRequestReceived, this.#handleIncomingVerification);
    }
    this.#notify();
  };

  #clearIncoming(): void {
    for (const [requestId] of this.#incoming) {
      this.#removeIncoming(requestId, false);
    }
  }

  #notify(): void {
    for (const listener of this.#listeners) {
      listener();
    }
  }

  #removeIncoming(requestId: string, notify = true): void {
    const pending = this.#incoming.get(requestId);
    if (pending === undefined) {
      return;
    }
    pending.request.off(VerificationRequestEvent.Change, pending.onChange);
    this.#incoming.delete(requestId);
    if (notify) {
      this.#notify();
    }
  }
}

type PendingVerification = {
  readonly notice: MatrixIncomingVerification;
  readonly onChange: () => void;
  readonly request: VerificationRequest;
};

type ActiveCryptoClient = {
  readonly client: MatrixClient;
  readonly crypto: CryptoApi;
  readonly deviceId: string;
  readonly userId: string;
};

function activeCryptoClient(
  client: MatrixClient | null,
): Result<ActiveCryptoClient, MatrixSecurityFailure> {
  if (client === null) {
    return err(failure('security.matrix_unavailable', true));
  }
  const crypto = client.getCrypto();
  if (crypto === undefined) {
    return err(failure('security.crypto_unavailable', true));
  }
  const userId = client.getUserId();
  const deviceId = client.getDeviceId();
  return userId === null || deviceId === null
    ? err(failure('security.identity_unavailable', false))
    : ok({ client, crypto, deviceId, userId });
}

async function decodeRecoveryCredential(
  credential: string,
  passphrase:
    { readonly bits?: number; readonly iterations: number; readonly salt: string } | undefined,
): Promise<Uint8Array<ArrayBuffer>> {
  const { decodeRecoveryKey, deriveRecoveryKeyFromPassphrase } =
    await import('matrix-js-sdk/lib/crypto-api/index.js');
  try {
    return decodeRecoveryKey(credential);
  } catch {
    if (passphrase === undefined) {
      throw new Error('恢复凭据不是当前 Secret Storage 的恢复密钥。');
    }
    return await deriveRecoveryKeyFromPassphrase(
      credential,
      passphrase.salt,
      passphrase.iterations,
      passphrase.bits,
    );
  }
}

function ignoreRecoveryProgress(progress: MatrixRecoveryProgress): void {
  void progress;
}

function incomingRequestId(request: VerificationRequest): string {
  return (
    request.transactionId ??
    `${request.otherUserId}\u0000${request.otherDeviceId ?? 'unknown-device'}`
  );
}

function isValidDeviceId(deviceId: string): boolean {
  return (
    deviceId.length > 0 &&
    deviceId.length <= 255 &&
    !deviceId.startsWith(' ') &&
    !deviceId.endsWith(' ') &&
    !/[\p{Cc}\p{Cf}]/u.test(deviceId)
  );
}

function participantUserIds(
  client: MatrixClient,
  roomId: string | undefined,
  userId: string,
): string[] {
  if (roomId === undefined) {
    return [userId];
  }
  const room = client.getRoom(roomId);
  if (room === null) {
    return [userId];
  }
  return [...new Set([userId, ...room.getJoinedMembers().map((member) => member.userId)])];
}

async function inspectBackup(crypto: CryptoApi): Promise<MatrixBackupState> {
  const backup = await crypto.checkKeyBackupAndEnable();
  if (backup === null) {
    return 'missing';
  }
  if (!backup.trustInfo.trusted) {
    return 'untrusted';
  }
  return backup.trustInfo.matchesDecryptionKey ? 'ready' : 'locked';
}

async function inspectRoomEncryption(
  crypto: CryptoApi,
  roomId: string | undefined,
): Promise<MatrixSecurityEvidence['roomEncryption']> {
  if (roomId === undefined) {
    return 'not_checked';
  }
  return (await crypto.isEncryptionEnabledInRoom(roomId)) ? 'encrypted' : 'unencrypted';
}

async function inspectDevices(
  crypto: CryptoApi,
  deviceMap: Map<string, Map<string, Device>>,
  currentUserId: string,
  currentDeviceId: string,
): Promise<readonly MatrixSecurityDevice[]> {
  const devices = [...deviceMap.entries()].flatMap(([userId, userDevices]) =>
    [...userDevices.values()].map(async (device) => {
      const status = await crypto.getDeviceVerificationStatus(userId, device.deviceId);
      const fingerprint = device.getFingerprint();
      return Object.freeze({
        current: userId === currentUserId && device.deviceId === currentDeviceId,
        deviceId: device.deviceId,
        ...(device.displayName === undefined ? {} : { displayName: device.displayName }),
        ...(fingerprint === undefined ? {} : { fingerprint }),
        trust: deviceTrust(status, userId === currentUserId),
        userId,
      });
    }),
  );
  return Object.freeze(
    (await Promise.all(devices)).toSorted((left, right) =>
      left.current === right.current
        ? left.userId.localeCompare(right.userId) || left.deviceId.localeCompare(right.deviceId)
        : left.current
          ? -1
          : 1,
    ),
  );
}

function deviceTrust(
  status: DeviceVerificationStatus | null,
  belongsToCurrentUser: boolean,
): MatrixDeviceTrust {
  if (status === null) {
    return 'unknown';
  }
  // Matrix 会默认把当前设备标记为“本地可信”。这只说明本机信任自己的设备密钥，
  // 不代表该设备已被账户的交叉签名身份签发，不能据此跳过跨设备验证。
  if (belongsToCurrentUser) {
    if (status.crossSigningVerified) {
      return 'verified';
    }
    return status.signedByOwner ? 'signed' : 'unverified';
  }
  if (status.isVerified()) {
    return 'verified';
  }
  return status.signedByOwner ? 'signed' : 'unverified';
}

function failure(code: MatrixSecurityFailure['code'], retryable: boolean): MatrixSecurityFailure {
  return Object.freeze({ code, retryable });
}
