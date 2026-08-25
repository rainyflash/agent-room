import { CircleAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { MatrixSecurityFailure } from '@/features/security/domain/matrix-security';
import { failureMessageKey } from '@/features/security/ui/security-copy';

export type SecurityFailureNoticeProps = {
  readonly failure: MatrixSecurityFailure;
};

export function SecurityFailureNotice({ failure }: SecurityFailureNoticeProps) {
  const { t } = useTranslation();
  return (
    <p className="security-failure" role="alert">
      <CircleAlert aria-hidden="true" />
      <span>{t(failureMessageKey[failure.code])}</span>
    </p>
  );
}
