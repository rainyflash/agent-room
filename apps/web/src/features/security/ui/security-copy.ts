import type {
  MatrixBackupState,
  MatrixSecurityFailure,
} from '@/features/security/domain/matrix-security';
import type { TranslationKey } from '@/shared/i18n/resources';

export const failureMessageKey: Readonly<Record<MatrixSecurityFailure['code'], TranslationKey>> = {
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

export const recoveryMessageKey: Readonly<Record<MatrixBackupState, TranslationKey>> = {
  locked: 'security.recovery.locked',
  missing: 'security.recovery.missing',
  ready: 'security.recovery.ready',
  untrusted: 'security.recovery.untrusted',
};
