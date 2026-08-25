import { Button } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ArrowLeft, LoaderCircle, RefreshCw, ShieldCheck } from 'lucide-react';
import { AnimatePresence, motion } from 'motion/react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import {
  matrixSecurityQueryKey,
  useMatrixSecurity,
} from '@/features/security/data/matrix-security-queries';
import type {
  MatrixSecurityBlocker,
  MatrixSecurityDevice,
  MatrixSecurityFailure,
  MatrixSecurityGateway,
  MatrixVerificationSession,
} from '@/features/security/domain/matrix-security';
import { DeviceVerificationDialog } from '@/features/security/ui/device-verification-dialog';
import { SecurityDeviceLedger } from '@/features/security/ui/security-device-ledger';
import { SecurityFailureNotice } from '@/features/security/ui/security-failure-notice';
import {
  SecurityPosture,
  type SecurityPostureAction,
} from '@/features/security/ui/security-posture';
import { SecurityRecoveryPanel } from '@/features/security/ui/security-recovery-panel';
import { LanguageControl } from '@/features/preferences/ui/language-control';

import './security-page.css';

export type SecurityPageProps = {
  readonly onBack: () => void;
};

export function SecurityPage({ onBack }: SecurityPageProps) {
  const { security } = useAppServices();
  return <SecurityWorkspace gateway={security} onBack={onBack} />;
}

export type SecurityWorkspaceProps = {
  readonly gateway: MatrixSecurityGateway;
  readonly onBack: () => void;
};

type ActiveVerification = {
  readonly session: MatrixVerificationSession;
  readonly targetName: string;
};

export function SecurityWorkspace({ gateway, onBack }: SecurityWorkspaceProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const inspection = useMatrixSecurity(gateway);
  const [verification, setVerification] = useState<ActiveVerification | null>(null);
  const beginVerification = useMutation({
    mutationFn: async (device: MatrixSecurityDevice) =>
      await gateway.beginVerification({ targetDeviceId: device.deviceId }),
    onSuccess: (result, device) => {
      if (!result.ok) {
        return;
      }
      setVerification({
        session: result.value,
        targetName: device.displayName ?? device.deviceId,
      });
    },
  });
  const establishIdentity = useMutation({
    mutationFn: async () => await gateway.establishIdentity(),
    onSuccess: (result) => {
      if (result.ok) {
        void queryClient.invalidateQueries({ queryKey: matrixSecurityQueryKey });
      }
    },
  });

  const refresh = (): void => {
    void queryClient.invalidateQueries({ queryKey: matrixSecurityQueryKey });
  };

  return (
    <main className="security-page" id="main-content">
      <header className="security-topbar">
        <Button
          icon={<ArrowLeft aria-hidden="true" />}
          onClick={onBack}
          size="compact"
          tone="quiet"
        >
          {t('security.action.back')}
        </Button>
        <a aria-label={t('app.name')} className="security-topbar__brand" href="/connect">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </a>
        <div className="security-topbar__actions">
          <button
            aria-label={t('security.action.refresh')}
            className="security-icon-button"
            disabled={inspection.isFetching}
            onClick={() => void inspection.refetch()}
            type="button"
          >
            <RefreshCw
              aria-hidden="true"
              className={inspection.isFetching ? 'security-spin' : undefined}
            />
          </button>
          <LanguageControl />
        </div>
      </header>

      <div className="security-workspace">
        <header className="security-page-heading">
          <div>
            <ShieldCheck aria-hidden="true" />
            <h1>{t('security.page.title')}</h1>
          </div>
          <p>{t('security.page.subtitle')}</p>
        </header>

        {inspection.isPending ? (
          <SecurityLoading />
        ) : inspection.data?.ok === false ? (
          <SecurityInspectionFailure
            failure={inspection.data.error}
            onRetry={() => void inspection.refetch()}
          />
        ) : inspection.data?.ok === true ? (
          <motion.div
            animate={{ opacity: 1 }}
            className="security-content"
            initial={{ opacity: 0 }}
            transition={{ duration: 0.18 }}
          >
            <div className="security-account-line">
              <div>
                <span>{t('security.identity.label')}</span>
                <strong>{inspection.data.value.userId}</strong>
              </div>
              <code>
                {t('security.identity.crypto', { version: inspection.data.value.cryptoVersion })}
              </code>
            </div>
            <div className="security-primary-grid">
              <SecurityPosture
                primaryAction={postureAction(
                  inspection.data.value,
                  establishIdentity.isPending,
                  () => establishIdentity.mutate(),
                  beginVerification.isPending,
                  (device) => beginVerification.mutate(device),
                  reviewRecovery,
                )}
                snapshot={inspection.data.value}
              />
              <SecurityDeviceLedger
                devices={inspection.data.value.devices}
                onVerify={(device) => beginVerification.mutate(device)}
                pendingDeviceId={
                  beginVerification.isPending ? beginVerification.variables.deviceId : null
                }
                verificationAvailable={inspection.data.value.crossSigningReady}
                verificationOpen={beginVerification.isPending || verification !== null}
              />
            </div>
            {beginVerification.data?.ok === false ? (
              <SecurityFailureNotice failure={beginVerification.data.error} />
            ) : null}
            {establishIdentity.data?.ok === false ? (
              <SecurityFailureNotice failure={establishIdentity.data.error} />
            ) : null}
            <SecurityRecoveryPanel
              gateway={gateway}
              onChanged={refresh}
              snapshot={inspection.data.value}
            />
          </motion.div>
        ) : (
          <SecurityInspectionFailure
            failure={{ code: 'security.inspection_failed', retryable: true }}
            onRetry={() => void inspection.refetch()}
          />
        )}
      </div>

      <AnimatePresence>
        {verification === null ? null : (
          <DeviceVerificationDialog
            key="device-verification"
            onClose={() => setVerification(null)}
            onVerified={refresh}
            session={verification.session}
            targetName={verification.targetName}
          />
        )}
      </AnimatePresence>
    </main>
  );
}

function SecurityLoading() {
  const { t } = useTranslation();
  return (
    <section aria-live="polite" className="security-boundary" role="status">
      <LoaderCircle aria-hidden="true" className="security-spin" />
      <h2>{t('security.loading.title')}</h2>
      <p>{t('security.loading.detail')}</p>
    </section>
  );
}

type SecurityInspectionFailureProps = {
  readonly failure: MatrixSecurityFailure;
  readonly onRetry: () => void;
};

function SecurityInspectionFailure({ failure, onRetry }: SecurityInspectionFailureProps) {
  const { t } = useTranslation();
  return (
    <section className="security-boundary">
      <ShieldCheck aria-hidden="true" />
      <h2>{t('security.inspectionFailed.title')}</h2>
      <SecurityFailureNotice failure={failure} />
      <Button icon={<RefreshCw aria-hidden="true" />} onClick={onRetry} tone="primary">
        {t('security.action.refresh')}
      </Button>
    </section>
  );
}

function reviewRecovery(): void {
  document
    .querySelector('#security-recovery')
    ?.scrollIntoView({ behavior: 'smooth', block: 'start' });
}

function postureAction(
  snapshot: {
    readonly blockers: readonly MatrixSecurityBlocker[];
    readonly crossSigningReady: boolean;
    readonly devices: readonly MatrixSecurityDevice[];
    readonly kind: 'action_required' | 'blocked' | 'ready';
  },
  identityPending: boolean,
  establishIdentity: () => void,
  verificationPending: boolean,
  verify: (device: MatrixSecurityDevice) => void,
  review: () => void,
): SecurityPostureAction | null {
  if (snapshot.kind === 'ready') {
    return null;
  }
  if (!snapshot.crossSigningReady) {
    return { kind: 'establish_identity', onSelect: establishIdentity, pending: identityPending };
  }
  const currentDevice = snapshot.devices.find((device) => device.current);
  if (snapshot.blockers.includes('current_device_unverified') && currentDevice !== undefined) {
    return {
      kind: 'verify_device',
      onSelect: () => verify(currentDevice),
      pending: verificationPending,
    };
  }
  return { kind: 'review_recovery', onSelect: review, pending: false };
}
