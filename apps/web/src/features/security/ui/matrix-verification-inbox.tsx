import { Button } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Fingerprint, ShieldCheck } from 'lucide-react';
import { motion } from 'motion/react';
import { useCallback, useState, useSyncExternalStore } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { matrixSecurityQueryKey } from '@/features/security/data/matrix-security-queries';
import type {
  MatrixIncomingVerification,
  MatrixSecurityGateway,
  MatrixVerificationSession,
} from '@/features/security/domain/matrix-security';
import { DeviceVerificationDialog } from '@/features/security/ui/device-verification-dialog';
import { SecurityFailureNotice } from '@/features/security/ui/security-failure-notice';

type ActiveVerification = {
  readonly session: MatrixVerificationSession;
  readonly targetName: string;
};

export function MatrixVerificationInbox() {
  const { security } = useAppServices();
  return <MatrixVerificationInboxView gateway={security} />;
}

export type MatrixVerificationInboxViewProps = {
  readonly gateway: MatrixSecurityGateway;
};

export function MatrixVerificationInboxView({ gateway }: MatrixVerificationInboxViewProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const subscribe = useCallback((listener: () => void) => gateway.subscribe(listener), [gateway]);
  const readIncoming = useCallback(() => gateway.getIncomingVerification(), [gateway]);
  const incoming = useSyncExternalStore(subscribe, readIncoming, noIncomingVerification);
  const [active, setActive] = useState<ActiveVerification | null>(null);
  const accept = useMutation({
    mutationFn: async (request: MatrixIncomingVerification) =>
      await gateway.acceptIncomingVerification(request.requestId),
    onSuccess: (result, request) => {
      if (!result.ok) {
        return;
      }
      setActive({
        session: result.value,
        targetName: request.sourceDeviceId ?? request.sourceUserId,
      });
    },
  });
  const decline = useMutation({
    mutationFn: async (request: MatrixIncomingVerification) =>
      await gateway.declineIncomingVerification(request.requestId),
  });

  if (active !== null) {
    return (
      <DeviceVerificationDialog
        onClose={() => {
          setActive(null);
        }}
        onVerified={() => {
          void queryClient.invalidateQueries({ queryKey: matrixSecurityQueryKey });
        }}
        session={active.session}
        targetName={active.targetName}
      />
    );
  }
  if (incoming === null) {
    return null;
  }

  return (
    <motion.aside
      animate={{ opacity: 1, x: 0, y: 0 }}
      aria-labelledby="matrix-verification-inbox-title"
      className="security-verification-inbox"
      initial={{ opacity: 0, x: 12, y: -8 }}
      role="dialog"
      transition={{ damping: 30, stiffness: 360, type: 'spring' }}
    >
      <div className="security-verification-inbox__icon">
        <Fingerprint aria-hidden="true" />
      </div>
      <div className="security-verification-inbox__copy">
        <span>{t('security.verification.incomingEyebrow')}</span>
        <h2 id="matrix-verification-inbox-title">{t('security.verification.incomingTitle')}</h2>
        <p>
          {t('security.verification.incomingDetail', {
            device: incoming.sourceDeviceId ?? incoming.sourceUserId,
          })}
        </p>
      </div>
      <div className="security-verification-inbox__actions">
        <Button
          disabled={accept.isPending || decline.isPending}
          onClick={() => {
            decline.mutate(incoming);
          }}
          size="compact"
          tone="quiet"
        >
          {t('security.verification.decline')}
        </Button>
        <Button
          disabled={accept.isPending || decline.isPending}
          icon={<ShieldCheck aria-hidden="true" />}
          onClick={() => {
            accept.mutate(incoming);
          }}
          size="compact"
          tone="primary"
        >
          {t('security.verification.accept')}
        </Button>
      </div>
      {accept.data?.ok === false ? <SecurityFailureNotice failure={accept.data.error} /> : null}
      {decline.data?.ok === false ? <SecurityFailureNotice failure={decline.data.error} /> : null}
    </motion.aside>
  );
}

function noIncomingVerification(): null {
  return null;
}
