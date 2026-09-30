import { Button, Toast } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Fingerprint, ShieldCheck } from 'lucide-react';
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
  readonly peer: boolean;
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
        peer: request.selfVerification === false,
        session: result.value,
        targetName:
          request.selfVerification === false
            ? `${request.sourceUserId} / ${request.sourceDeviceId ?? ''}`
            : (request.sourceDeviceId ?? request.sourceUserId),
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
        peer={active.peer}
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

  // 提示栈里一条：谁在请求核对，接受还是拒绝。接受后打开核对对话框。
  return (
    <Toast
      action={
        <>
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
        </>
      }
      className="security-verification-toast"
      icon={<Fingerprint />}
      role="alert"
      title={t(
        incoming.selfVerification === false
          ? 'security.verification.peerTitle'
          : 'security.verification.incomingTitle',
      )}
      tone="warning"
    >
      <p>
        {t(
          incoming.selfVerification === false
            ? 'security.verification.peerDetail'
            : 'security.verification.incomingDetail',
          {
            user: incoming.sourceUserId,
            device: incoming.sourceDeviceId ?? incoming.sourceUserId,
          },
        )}
      </p>
      {accept.data?.ok === false ? <SecurityFailureNotice failure={accept.data.error} /> : null}
      {decline.data?.ok === false ? <SecurityFailureNotice failure={decline.data.error} /> : null}
    </Toast>
  );
}

function noIncomingVerification(): null {
  return null;
}
