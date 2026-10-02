import type { DeviceVerificationStatus } from 'matrix-js-sdk/lib/crypto-api/index.js';

import type {
  MatrixDeviceTrust,
  MatrixSecurityDevice,
  MatrixSecurityFailure,
  MatrixSecurityGateway,
  MatrixSecuritySnapshot,
} from '@/features/security/domain/matrix-security';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';
import { err, ok, type Result } from '@/shared/result';

/** 读你的设备和它们签没签名。签名本身由登录后的自动签名负责（ADR 0011）。 */
export class MatrixSdkSecurityGateway implements MatrixSecurityGateway {
  readonly #clients: MatrixClientSource;
  readonly #listeners = new Set<() => void>();

  constructor(clients: MatrixClientSource) {
    this.#clients = clients;
    // 换了客户端，或者客户端有了新动静（比如自动签名刚跑完），都让界面重读一遍。
    this.#clients.subscribe(() => {
      for (const listener of this.#listeners) listener();
    });
  }

  async inspect(): Promise<Result<MatrixSecuritySnapshot, MatrixSecurityFailure>> {
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
      const deviceMap = await crypto.getUserDeviceInfo([userId], true);
      const devices = await Promise.all(
        [...(deviceMap.get(userId)?.values() ?? [])].map(
          async (device): Promise<MatrixSecurityDevice> => {
            const status = await crypto.getDeviceVerificationStatus(userId, device.deviceId);
            const fingerprint = device.getFingerprint();
            return Object.freeze({
              current: device.deviceId === deviceId,
              deviceId: device.deviceId,
              ...(device.displayName === undefined ? {} : { displayName: device.displayName }),
              ...(fingerprint === undefined ? {} : { fingerprint }),
              trust: deviceTrust(status),
              userId,
            });
          },
        ),
      );
      return ok(
        Object.freeze({
          currentDeviceId: deviceId,
          devices: Object.freeze(
            devices.toSorted((left, right) =>
              left.current === right.current
                ? left.deviceId.localeCompare(right.deviceId)
                : left.current
                  ? -1
                  : 1,
            ),
          ),
          userId,
        }),
      );
    } catch {
      return err(failure('security.inspection_failed', true));
    }
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }
}

function deviceTrust(status: DeviceVerificationStatus | null): MatrixDeviceTrust {
  if (status === null) {
    return 'unknown';
  }
  // Matrix 默认把这台设备标成“本地可信”，那只说明本机信任自己的设备密钥，不代表它由账户的
  // 签名身份签过，所以只看签名。
  if (status.crossSigningVerified) {
    return 'verified';
  }
  return status.signedByOwner ? 'signed' : 'unverified';
}

function failure(code: MatrixSecurityFailure['code'], retryable: boolean): MatrixSecurityFailure {
  return Object.freeze({ code, retryable });
}
