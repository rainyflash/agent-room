import { StatusMark, type StatusTone } from '@agent-room/ui-system';
import { Cloud, Network, PlugZap } from 'lucide-react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import type { FleetInstanceStatus } from '@/features/workspace/domain/agent-fleet';

export type WorkspaceStatusValue = FleetInstanceStatus | 'unavailable';

const STATUS_TONE: Readonly<Record<WorkspaceStatusValue, StatusTone>> = {
  connecting: 'network',
  degraded: 'alert',
  offline: 'offline',
  online: 'active',
  revoked: 'alert',
  unavailable: 'idle',
};

export function ConnectionStatusStrip({
  bridgeStatus,
}: {
  readonly bridgeStatus: WorkspaceStatusValue;
}) {
  const { t } = useTranslation();
  return (
    <section aria-label={t('workspace.connectionStatus.title')} className="workspace-status-strip">
      <ConnectionStatus
        icon={<Cloud aria-hidden="true" />}
        label={t('workspace.cloud.control')}
        status="online"
      />
      <ConnectionStatus
        icon={<Network aria-hidden="true" />}
        label={t('workspace.cloud.matrix')}
        status="online"
      />
      <ConnectionStatus
        icon={<PlugZap aria-hidden="true" />}
        label={t('workspace.local.bridge')}
        status={bridgeStatus}
      />
    </section>
  );
}

export function bridgeWorkspaceStatus(
  available: boolean,
  phase: string | undefined,
): WorkspaceStatusValue {
  if (!available) return 'unavailable';
  const statuses: Readonly<Record<string, WorkspaceStatusValue>> = {
    authorization_required: 'connecting',
    authorized: 'connecting',
    discovering: 'connecting',
    halted: 'degraded',
    ready: 'online',
    retry_scheduled: 'degraded',
    starting: 'connecting',
    stopped: 'offline',
  };
  return phase === undefined ? 'connecting' : (statuses[phase] ?? 'degraded');
}

function ConnectionStatus({
  icon,
  label,
  status,
}: {
  readonly icon: ReactNode;
  readonly label: string;
  readonly status: WorkspaceStatusValue;
}) {
  const { t } = useTranslation();
  return (
    <div className="workspace-status">
      <span>{icon}</span>
      <div>
        <small>{label}</small>
        <strong>{t(`workspace.status.${status}`)}</strong>
      </div>
      <StatusMark
        label={t(`workspace.status.${status}`)}
        pulse={status === 'connecting'}
        tone={STATUS_TONE[status]}
      />
    </div>
  );
}
