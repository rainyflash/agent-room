import { Button, Details } from '@agent-room/ui-system';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { LoaderCircle, RefreshCw, ShieldCheck } from 'lucide-react';
import { AnimatePresence, motion } from 'motion/react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import {
  matrixSecurityQueryKey,
  useMatrixSecurity,
} from '@/features/security/data/matrix-security-queries';
import type { AccessManagementGateway } from '@/features/security/domain/access-management';
import type {
  MatrixSecurityDevice,
  MatrixSecurityFailure,
  MatrixSecurityGateway,
  MatrixSecuritySnapshot,
  MatrixVerificationSession,
} from '@/features/security/domain/matrix-security';
import { AccessManagementLedger } from '@/features/security/ui/access-management-ledger';
import { DeviceVerificationDialog } from '@/features/security/ui/device-verification-dialog';
import { SecurityDeviceLedger } from '@/features/security/ui/security-device-ledger';
import { SecurityFailureNotice } from '@/features/security/ui/security-failure-notice';
import {
  SecurityRecoveryPanel,
  type RecoveryMode,
} from '@/features/security/ui/security-recovery-panel';
import { thisDeviceSigning } from '@/features/security/domain/device-signing';
import { ThisDeviceSigning } from '@/features/security/ui/this-device-signing';
import type { AgentRecoveryGateway } from '@/features/security/domain/agent-recovery';
import { AgentRecoveryPanel } from '@/features/security/ui/agent-recovery-panel';

import './security-page.css';

/** “设置 → 安全”：这台设备的签名、你的设备、恢复密钥、已授权的电脑和 Agent。 */
export function SecuritySettings() {
  const { accessManagement, security, localRuntime } = useAppServices();
  return (
    <SecurityWorkspace
      accessManagement={accessManagement}
      gateway={security}
      {...(localRuntime.isAvailable() && localRuntime.agentRecovery
        ? { agentRecovery: localRuntime.agentRecovery }
        : {})}
    />
  );
}

export type SecurityWorkspaceProps = {
  readonly agentRecovery?: AgentRecoveryGateway;
  readonly accessManagement: AccessManagementGateway;
  readonly gateway: MatrixSecurityGateway;
};

type ActiveVerification = {
  readonly session: MatrixVerificationSession;
  readonly targetName: string;
};

export function SecurityWorkspace({
  accessManagement,
  gateway,
  agentRecovery,
}: SecurityWorkspaceProps) {
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
    <div className="security-page">
      <div className="security-workspace">
        <header className="security-page-heading">
          <p>{t('security.page.subtitle')}</p>
          <button
            aria-label={t('security.action.refresh')}
            className="security-icon-button ar-icon-button"
            disabled={inspection.isFetching}
            onClick={() => void inspection.refetch()}
            type="button"
          >
            <RefreshCw
              aria-hidden="true"
              className={inspection.isFetching ? 'security-spin' : undefined}
            />
          </button>
        </header>

        {inspection.isPending ? (
          <SecurityLoading />
        ) : inspection.data?.ok === false ? (
          <SecurityInspectionFailure
            failure={inspection.data.error}
            onRetry={() => void inspection.refetch()}
          />
        ) : inspection.data?.ok === true ? (
          <SecurityContent
            beginVerification={(device) => {
              beginVerification.mutate(device);
            }}
            establishIdentity={() => {
              establishIdentity.mutate();
            }}
            gateway={gateway}
            identityFailure={
              establishIdentity.data?.ok === false ? establishIdentity.data.error : null
            }
            identityPending={establishIdentity.isPending}
            onChanged={refresh}
            snapshot={inspection.data.value}
            verificationFailure={
              beginVerification.data?.ok === false ? beginVerification.data.error : null
            }
            verificationOpen={beginVerification.isPending || verification !== null}
            verificationPendingId={
              beginVerification.isPending ? beginVerification.variables.deviceId : null
            }
          />
        ) : (
          <SecurityInspectionFailure
            failure={{ code: 'security.inspection_failed', retryable: true }}
            onRetry={() => void inspection.refetch()}
          />
        )}
        {agentRecovery === undefined ? null : <AgentRecoveryPanel gateway={agentRecovery} />}
        <AccessManagementLedger gateway={accessManagement} />
      </div>

      <AnimatePresence>
        {verification === null ? null : (
          <DeviceVerificationDialog
            key="device-verification"
            onClose={() => {
              setVerification(null);
            }}
            onVerified={refresh}
            session={verification.session}
            targetName={verification.targetName}
          />
        )}
      </AnimatePresence>
    </div>
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

type SecurityContentProps = {
  readonly gateway: MatrixSecurityGateway;
  readonly snapshot: MatrixSecuritySnapshot;
  readonly identityPending: boolean;
  readonly identityFailure: MatrixSecurityFailure | null;
  readonly verificationOpen: boolean;
  readonly verificationPendingId: string | null;
  readonly verificationFailure: MatrixSecurityFailure | null;
  readonly establishIdentity: () => void;
  readonly beginVerification: (device: MatrixSecurityDevice) => void;
  readonly onChanged: () => void;
};

/**
 * 安全一节的内容：这台设备签没签名（要签就给两条路）、你的设备、恢复密钥；账户的 Matrix ID
 * 收在最下面的详情里。
 */
function SecurityContent({
  gateway,
  snapshot,
  identityPending,
  identityFailure,
  verificationOpen,
  verificationPendingId,
  verificationFailure,
  establishIdentity,
  beginVerification,
  onChanged,
}: SecurityContentProps) {
  const { t } = useTranslation();
  // 点“输入恢复密钥”时让恢复密钥一节直接打开输入框；每点一次换一个 key 重新打开。
  const [recoveryRequest, setRecoveryRequest] = useState<{
    readonly mode: RecoveryMode;
    readonly key: number;
  } | null>(null);
  const signing = thisDeviceSigning(snapshot);
  const currentDevice = snapshot.devices.find((device) => device.current);
  const otherDevices = snapshot.devices.some((device) => !device.current);
  return (
    <motion.div
      animate={{ opacity: 1 }}
      className="security-content"
      id="security-identity"
      initial={{ opacity: 0 }}
      transition={{ duration: 0.18 }}
    >
      <ThisDeviceSigning
        identityPending={identityPending}
        onEstablishIdentity={establishIdentity}
        onUseRecoveryKey={() => {
          setRecoveryRequest((previous) => ({ mode: 'recover', key: (previous?.key ?? 0) + 1 }));
          document
            .querySelector('#security-recovery')
            ?.scrollIntoView({ behavior: 'smooth', block: 'start' });
        }}
        onVerifyWithOtherDevice={() => {
          if (currentDevice !== undefined) beginVerification(currentDevice);
        }}
        otherDevices={otherDevices}
        state={signing}
        verificationPending={verificationOpen}
      />
      {identityFailure === null ? null : <SecurityFailureNotice failure={identityFailure} />}
      {verificationFailure === null ? null : (
        <SecurityFailureNotice failure={verificationFailure} />
      )}
      <SecurityDeviceLedger
        devices={snapshot.devices}
        onVerify={beginVerification}
        pendingDeviceId={verificationPendingId}
        thisDeviceSigned={signing === 'signed'}
        verificationAvailable={snapshot.crossSigningIdentityExists}
        verificationOpen={verificationOpen}
      />
      <SecurityRecoveryPanel
        gateway={gateway}
        key={recoveryRequest?.key ?? 0}
        onChanged={onChanged}
        openMode={recoveryRequest?.mode ?? null}
        snapshot={snapshot}
      />
      <Details className="security-account-details" summary={t('security.account.details')}>
        <dl>
          <div>
            <dt>{t('security.account.matrixId')}</dt>
            <dd>
              <code>{snapshot.userId}</code>
            </dd>
          </div>
        </dl>
      </Details>
    </motion.div>
  );
}
