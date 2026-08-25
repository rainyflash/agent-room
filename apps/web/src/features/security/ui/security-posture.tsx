import { Button } from '@agent-room/ui-system';
import { KeyRound, ShieldAlert, ShieldCheck, ShieldX, type LucideIcon } from 'lucide-react';
import { motion } from 'motion/react';
import { useTranslation } from 'react-i18next';

import type { MatrixSecuritySnapshot } from '@/features/security/domain/matrix-security';
import {
  blockerMessageKey,
  postureDetailKey,
  postureTitleKey,
} from '@/features/security/ui/security-copy';

export type SecurityPostureProps = {
  readonly onReviewRecovery: () => void;
  readonly onVerifyCurrent: (() => void) | undefined;
  readonly snapshot: MatrixSecuritySnapshot;
  readonly verificationPending: boolean;
};

const postureIcon: Readonly<Record<MatrixSecuritySnapshot['kind'], LucideIcon>> = {
  action_required: ShieldAlert,
  blocked: ShieldX,
  ready: ShieldCheck,
};

export function SecurityPosture({
  onReviewRecovery,
  onVerifyCurrent,
  snapshot,
  verificationPending,
}: SecurityPostureProps) {
  const { t } = useTranslation();
  const PostureIcon = postureIcon[snapshot.kind];
  const requiresVerification = onVerifyCurrent !== undefined;

  return (
    <motion.section
      animate={{ opacity: 1, y: 0 }}
      className={`security-posture security-posture--${snapshot.kind}`}
      initial={{ opacity: 0, y: 10 }}
      transition={{ damping: 28, stiffness: 300, type: 'spring' }}
    >
      <div className="security-posture__signal">
        <PostureIcon aria-hidden="true" />
      </div>
      <div className="security-posture__copy">
        <h2>{t(postureTitleKey[snapshot.kind])}</h2>
        <p>{t(postureDetailKey[snapshot.kind])}</p>
      </div>
      <div className="security-posture__facts">
        <span className={snapshot.sendAllowed ? 'is-allowed' : 'is-blocked'}>
          {t(
            snapshot.sendAllowed ? 'security.posture.sendAllowed' : 'security.posture.sendBlocked',
          )}
        </span>
        {snapshot.excludedDeviceCount === 0 ? null : (
          <span>
            {t('security.posture.excludedDevices', { count: snapshot.excludedDeviceCount })}
          </span>
        )}
      </div>
      {snapshot.blockers.length === 0 ? null : (
        <ul className="security-posture__blockers">
          {snapshot.blockers.map((blocker) => (
            <li key={blocker}>{t(blockerMessageKey[blocker])}</li>
          ))}
        </ul>
      )}
      {snapshot.kind === 'ready' ? null : (
        <Button
          disabled={verificationPending}
          icon={<KeyRound aria-hidden="true" />}
          onClick={requiresVerification ? onVerifyCurrent : onReviewRecovery}
          size="large"
          tone={requiresVerification ? 'primary' : 'ghost'}
        >
          {t(requiresVerification ? 'security.devices.verify' : 'security.recovery.review')}
        </Button>
      )}
    </motion.section>
  );
}
