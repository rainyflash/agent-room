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
import type { TranslationKey } from '@/shared/i18n/resources';

export type SecurityPostureProps = {
  readonly primaryAction: SecurityPostureAction | null;
  readonly snapshot: MatrixSecuritySnapshot;
};

export type SecurityPostureAction = {
  readonly kind: 'establish_identity' | 'review_recovery' | 'verify_device';
  readonly onSelect: () => void;
  readonly pending: boolean;
};

const postureIcon: Readonly<Record<MatrixSecuritySnapshot['kind'], LucideIcon>> = {
  action_required: ShieldAlert,
  blocked: ShieldX,
  ready: ShieldCheck,
};

const actionPresentation: Readonly<
  Record<
    SecurityPostureAction['kind'],
    { readonly label: TranslationKey; readonly tone: 'ghost' | 'primary' }
  >
> = {
  establish_identity: { label: 'security.identity.establish', tone: 'primary' },
  review_recovery: { label: 'security.recovery.review', tone: 'ghost' },
  verify_device: { label: 'security.devices.verify', tone: 'primary' },
};

export function SecurityPosture({ primaryAction, snapshot }: SecurityPostureProps) {
  const { t } = useTranslation();
  const PostureIcon = postureIcon[snapshot.kind];

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
      {primaryAction === null ? null : (
        <Button
          disabled={primaryAction.pending}
          icon={<KeyRound aria-hidden="true" />}
          onClick={primaryAction.onSelect}
          size="large"
          tone={actionPresentation[primaryAction.kind].tone}
        >
          {t(actionPresentation[primaryAction.kind].label)}
        </Button>
      )}
    </motion.section>
  );
}
