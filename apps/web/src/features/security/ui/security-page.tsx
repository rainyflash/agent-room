import { Button, Details } from '@agent-room/ui-system';
import { LoaderCircle, RefreshCw, ShieldCheck } from 'lucide-react';
import { motion } from 'motion/react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { useDeviceSigningState } from '@/features/security/data/device-signing-state';
import { useMatrixSecurity } from '@/features/security/data/matrix-security-queries';
import type { AccessManagementGateway } from '@/features/security/domain/access-management';
import type { AgentRecoveryGateway } from '@/features/security/domain/agent-recovery';
import { thisDeviceState } from '@/features/security/domain/device-signing';
import type {
  MatrixSecurityFailure,
  MatrixSecurityGateway,
  MatrixSecuritySnapshot,
} from '@/features/security/domain/matrix-security';
import { AccessManagementLedger } from '@/features/security/ui/access-management-ledger';
import type { Reauthentication } from '@/features/security/ui/bulk-device-revocation-dialog';
import { AgentRecoveryPanel } from '@/features/security/ui/agent-recovery-panel';
import { SecurityDeviceLedger } from '@/features/security/ui/security-device-ledger';
import { SecurityFailureNotice } from '@/features/security/ui/security-failure-notice';
import { ThisDeviceStatus } from '@/features/security/ui/this-device-status';
import { useOptionalSession } from '@/features/session/ui/session-provider';
import type { DeviceSigningStatus } from '@/shared/matrix/device-signing-status';

import './security-page.css';

/** 撤销电脑前重新登录完回到这里。 */
const SECURITY_PATH = '/settings/security';

/** “设置 → 安全”：这台设备准备好没有、你的设备、已授权的电脑和 Agent。 */
export function SecuritySettings() {
  const { accessManagement, controlPlane, deviceSigning, security, localRuntime } =
    useAppServices();
  const principal = useOptionalSession()?.snapshot.context.principal ?? null;
  return (
    <SecurityWorkspace
      accessManagement={accessManagement}
      deviceSigning={deviceSigning}
      gateway={security}
      {...(principal === null
        ? {}
        : {
            reauthentication: {
              recentlyAuthenticated: principal.recentlyAuthenticated,
              signInAgain: () => {
                void controlPlane.beginAuthentication(SECURITY_PATH);
              },
            },
          })}
      {...(localRuntime.isAvailable() && localRuntime.agentRecovery
        ? { agentRecovery: localRuntime.agentRecovery }
        : {})}
    />
  );
}

export type SecurityWorkspaceProps = {
  readonly agentRecovery?: AgentRecoveryGateway;
  readonly accessManagement: AccessManagementGateway;
  /** 这台设备的自动签名（ADR 0011）：准备中、就绪、出错可重试。 */
  readonly deviceSigning: DeviceSigningStatus;
  readonly gateway: MatrixSecurityGateway;
  /** 撤销电脑要最近登录过：登录着的时候给，没登录（测试）就不先拦着。 */
  readonly reauthentication?: Reauthentication;
};

export function SecurityWorkspace({
  accessManagement,
  deviceSigning,
  gateway,
  agentRecovery,
  reauthentication,
}: SecurityWorkspaceProps) {
  const { t } = useTranslation();
  const inspection = useMatrixSecurity(gateway);
  const signing = useDeviceSigningState(deviceSigning);
  const snapshot = inspection.data?.ok === true ? inspection.data.value : null;
  const state = thisDeviceState(snapshot, signing);

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

        <ThisDeviceStatus
          onRetry={() => {
            deviceSigning.retry();
          }}
          state={state}
        />
        {inspection.isPending ? (
          <SecurityLoading />
        ) : inspection.data?.ok === false ? (
          <SecurityInspectionFailure
            failure={inspection.data.error}
            onRetry={() => void inspection.refetch()}
          />
        ) : snapshot === null ? (
          <SecurityInspectionFailure
            failure={{ code: 'security.inspection_failed', retryable: true }}
            onRetry={() => void inspection.refetch()}
          />
        ) : (
          <SecurityContent snapshot={snapshot} thisDeviceReady={state === 'ready'} />
        )}
        {agentRecovery === undefined ? null : <AgentRecoveryPanel gateway={agentRecovery} />}
        <AccessManagementLedger gateway={accessManagement} reauthentication={reauthentication} />
      </div>
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

/** 你的设备，以及收在最下面详情里的 Matrix ID。 */
function SecurityContent({
  snapshot,
  thisDeviceReady,
}: {
  readonly snapshot: MatrixSecuritySnapshot;
  readonly thisDeviceReady: boolean;
}) {
  const { t } = useTranslation();
  return (
    <motion.div
      animate={{ opacity: 1 }}
      className="security-content"
      id="security-identity"
      initial={{ opacity: 0 }}
      transition={{ duration: 0.18 }}
    >
      <SecurityDeviceLedger devices={snapshot.devices} thisDeviceSigned={thisDeviceReady} />
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
