import type { CryptoApi, DeviceVerificationStatus } from 'matrix-js-sdk/lib/crypto-api/index.js';
import type { Device, MatrixClient } from 'matrix-js-sdk';

import {
  evaluateMatrixSecurity,
  type MatrixBackupState,
  type MatrixDeviceTrust,
  type MatrixSecurityDevice,
  type MatrixSecurityEvidence,
  type MatrixSecurityFailure,
  type MatrixSecurityGateway,
  type MatrixSecurityInspection,
  type MatrixSecuritySnapshot,
} from '@/features/security/domain/matrix-security';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import { err, ok, type Result } from '@/shared/result';

export class MatrixSdkSecurityGateway implements MatrixSecurityGateway {
  readonly #clients: MatrixClientSource;

  constructor(clients: MatrixClientSource) {
    this.#clients = clients;
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
      const [crossSigningReady, secretStorage, backup, roomEncryption, deviceMap] =
        await Promise.all([
          crypto.isCrossSigningReady(),
          crypto.getSecretStorageStatus(),
          inspectBackup(crypto),
          inspectRoomEncryption(crypto, inspection.roomId),
          crypto.getUserDeviceInfo(userIds, true),
        ]);
      const devices = await inspectDevices(crypto, deviceMap, userId, deviceId);
      const evidence: MatrixSecurityEvidence = Object.freeze({
        backup,
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

  subscribe(listener: () => void): () => void {
    return this.#clients.subscribe(listener);
  }
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
        trust: deviceTrust(status),
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

function deviceTrust(status: DeviceVerificationStatus | null): MatrixDeviceTrust {
  if (status === null) {
    return 'unknown';
  }
  if (status.isVerified()) {
    return 'verified';
  }
  return status.signedByOwner ? 'signed' : 'unverified';
}

function failure(code: MatrixSecurityFailure['code'], retryable: boolean): MatrixSecurityFailure {
  return Object.freeze({ code, retryable });
}
