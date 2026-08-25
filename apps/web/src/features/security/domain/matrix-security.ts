import type { Result } from '@/shared/result';

export type MatrixDeviceTrust = 'signed' | 'unknown' | 'unverified' | 'verified';
export type MatrixBackupState = 'locked' | 'missing' | 'ready' | 'untrusted';
export type MatrixRoomEncryptionState = 'encrypted' | 'not_checked' | 'unencrypted';

export type MatrixSecurityBlocker =
  | 'backup_locked'
  | 'backup_missing'
  | 'backup_untrusted'
  | 'cross_signing_missing'
  | 'current_device_unverified'
  | 'room_unencrypted'
  | 'secret_storage_missing';

export type MatrixSecurityDevice = {
  readonly current: boolean;
  readonly deviceId: string;
  readonly displayName?: string;
  readonly fingerprint?: string;
  readonly trust: MatrixDeviceTrust;
  readonly userId: string;
};

export type MatrixSecurityEvidence = {
  readonly backup: MatrixBackupState;
  readonly crossSigningReady: boolean;
  readonly cryptoVersion: string;
  readonly currentDeviceId: string;
  readonly devices: readonly MatrixSecurityDevice[];
  readonly roomEncryption: MatrixRoomEncryptionState;
  readonly roomId?: string;
  readonly secretStorageReady: boolean;
  readonly userId: string;
};

export type MatrixSecurityPosture = {
  readonly blockers: readonly MatrixSecurityBlocker[];
  readonly excludedDeviceCount: number;
  readonly kind: 'action_required' | 'blocked' | 'ready';
  readonly sendAllowed: boolean;
};

export type MatrixSecuritySnapshot = MatrixSecurityEvidence & MatrixSecurityPosture;

export type MatrixSecurityFailure = {
  readonly code:
    | 'security.crypto_unavailable'
    | 'security.identity_unavailable'
    | 'security.inspection_failed'
    | 'security.matrix_unavailable';
  readonly retryable: boolean;
};

export type MatrixSecurityInspection = {
  readonly roomId?: string;
};

export type MatrixSecurityGateway = {
  inspect(
    inspection?: MatrixSecurityInspection,
  ): Promise<Result<MatrixSecuritySnapshot, MatrixSecurityFailure>>;
  subscribe(listener: () => void): () => void;
};

const recoveryBlockers = new Set<MatrixSecurityBlocker>([
  'backup_locked',
  'backup_missing',
  'backup_untrusted',
  'secret_storage_missing',
]);

export function evaluateMatrixSecurity(evidence: MatrixSecurityEvidence): MatrixSecurityPosture {
  const currentDevice = evidence.devices.find((device) => device.current);
  const blockers = Object.freeze([
    ...(evidence.crossSigningReady ? [] : (['cross_signing_missing'] as const)),
    ...(currentDevice?.trust === 'verified' ? [] : (['current_device_unverified'] as const)),
    ...(evidence.secretStorageReady ? [] : (['secret_storage_missing'] as const)),
    ...backupBlockers[evidence.backup],
    ...(evidence.roomEncryption === 'unencrypted' ? (['room_unencrypted'] as const) : []),
  ] satisfies readonly MatrixSecurityBlocker[]);
  const excludedDeviceCount = evidence.devices.filter(
    (device) => !device.current && device.trust !== 'signed' && device.trust !== 'verified',
  ).length;
  const sendAllowed = blockers.every((blocker) => recoveryBlockers.has(blocker));

  return Object.freeze({
    blockers,
    excludedDeviceCount,
    kind: sendAllowed ? (blockers.length === 0 ? 'ready' : 'action_required') : 'blocked',
    sendAllowed,
  });
}

const backupBlockers: Readonly<Record<MatrixBackupState, readonly MatrixSecurityBlocker[]>> = {
  locked: ['backup_locked'],
  missing: ['backup_missing'],
  ready: [],
  untrusted: ['backup_untrusted'],
};
