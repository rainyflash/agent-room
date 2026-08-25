import type {
  MatrixBackupState,
  MatrixDeviceTrust,
  MatrixSecurityBlocker,
  MatrixSecurityFailure,
  MatrixSecurityPosture,
} from '@/features/security/domain/matrix-security';

export const blockerMessageKey: Readonly<Record<MatrixSecurityBlocker, string>> = {
  backup_locked: 'security.blocker.backup_locked',
  backup_missing: 'security.blocker.backup_missing',
  backup_untrusted: 'security.blocker.backup_untrusted',
  cross_signing_missing: 'security.blocker.cross_signing_missing',
  cross_signing_not_ready: 'security.blocker.cross_signing_not_ready',
  current_device_unverified: 'security.blocker.current_device_unverified',
  room_unencrypted: 'security.blocker.room_unencrypted',
  secret_storage_missing: 'security.blocker.secret_storage_missing',
};

export const failureMessageKey: Readonly<Record<MatrixSecurityFailure['code'], string>> = {
  'security.crypto_unavailable': 'security.failure.crypto_unavailable',
  'security.identity_bootstrap_failed': 'security.failure.identity_bootstrap_failed',
  'security.identity_unavailable': 'security.failure.identity_unavailable',
  'security.inspection_failed': 'security.failure.inspection_failed',
  'security.matrix_unavailable': 'security.failure.matrix_unavailable',
  'security.recovery_already_configured': 'security.failure.recovery_already_configured',
  'security.recovery_credential_invalid': 'security.failure.recovery_credential_invalid',
  'security.recovery_failed': 'security.failure.recovery_failed',
  'security.recovery_key_missing': 'security.failure.recovery_key_missing',
  'security.recovery_key_rejected': 'security.failure.recovery_key_rejected',
  'security.recovery_setup_failed': 'security.failure.recovery_setup_failed',
  'security.verification_failed': 'security.failure.verification_failed',
  'security.verification_required': 'security.failure.verification_required',
  'security.verification_unavailable': 'security.failure.verification_unavailable',
};

export const trustMessageKey: Readonly<Record<MatrixDeviceTrust, string>> = {
  signed: 'security.trust.signed',
  unknown: 'security.trust.unknown',
  unverified: 'security.trust.unverified',
  verified: 'security.trust.verified',
};

export const recoveryMessageKey: Readonly<Record<MatrixBackupState, string>> = {
  locked: 'security.recovery.locked',
  missing: 'security.recovery.missing',
  ready: 'security.recovery.ready',
  untrusted: 'security.recovery.untrusted',
};

export const postureTitleKey: Readonly<Record<MatrixSecurityPosture['kind'], string>> = {
  action_required: 'security.posture.action_required.title',
  blocked: 'security.posture.blocked.title',
  ready: 'security.posture.ready.title',
};

export const postureDetailKey: Readonly<Record<MatrixSecurityPosture['kind'], string>> = {
  action_required: 'security.posture.action_required.detail',
  blocked: 'security.posture.blocked.detail',
  ready: 'security.posture.ready.detail',
};
