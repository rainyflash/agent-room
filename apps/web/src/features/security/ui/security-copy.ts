import type { MatrixSecurityFailure } from '@/features/security/domain/matrix-security';
import type { TranslationKey } from '@/shared/i18n/resources';

export const failureMessageKey: Readonly<Record<MatrixSecurityFailure['code'], TranslationKey>> = {
  'security.crypto_unavailable': 'security.failure.crypto_unavailable',
  'security.identity_unavailable': 'security.failure.identity_unavailable',
  'security.inspection_failed': 'security.failure.inspection_failed',
  'security.matrix_unavailable': 'security.failure.matrix_unavailable',
};
