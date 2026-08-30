import { StatusMark, type StatusTone } from '@agent-room/ui-system';
import { Bot, Cloud, Network, PlugZap } from 'lucide-react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import type {
  WorkspaceConnectionHealth,
  WorkspaceLayerId,
  WorkspaceLayerStatus,
} from '@/features/workspace/domain/connection-health';
import { formatWorkspaceTime } from '@/features/workspace/ui/workspace-format';

const STATUS_TONE: Readonly<Record<WorkspaceLayerStatus, StatusTone>> = {
  connecting: 'network',
  degraded: 'alert',
  offline: 'offline',
  online: 'active',
  revoked: 'alert',
  unavailable: 'idle',
};

export const WORKSPACE_LAYER_ORDER = [
  'controlPlane',
  'matrix',
  'bridge',
  'agents',
] as const satisfies readonly WorkspaceLayerId[];

const LAYER_PRESENTATION = {
  agents: { icon: <Bot aria-hidden="true" />, labelKey: 'workspace.agents.runtime' },
  bridge: { icon: <PlugZap aria-hidden="true" />, labelKey: 'workspace.local.bridge' },
  controlPlane: { icon: <Cloud aria-hidden="true" />, labelKey: 'workspace.cloud.control' },
  matrix: { icon: <Network aria-hidden="true" />, labelKey: 'workspace.cloud.matrix' },
} as const satisfies Readonly<
  Record<WorkspaceLayerId, { readonly icon: ReactNode; readonly labelKey: string }>
>;

export function ConnectionStatusStrip({ health }: { readonly health: WorkspaceConnectionHealth }) {
  const { i18n, t } = useTranslation();
  return (
    <section aria-label={t('workspace.connectionStatus.title')} className="workspace-status-strip">
      {WORKSPACE_LAYER_ORDER.map((layerId) => {
        const presentation = LAYER_PRESENTATION[layerId];
        return (
          <ConnectionStatus
            health={health[layerId]}
            icon={presentation.icon}
            key={layerId}
            label={t(presentation.labelKey)}
            language={i18n.resolvedLanguage}
          />
        );
      })}
    </section>
  );
}

export function workspaceStatusTone(status: WorkspaceLayerStatus): StatusTone {
  return STATUS_TONE[status];
}

function ConnectionStatus({
  health,
  icon,
  label,
  language,
}: {
  readonly health: WorkspaceConnectionHealth[WorkspaceLayerId];
  readonly icon: ReactNode;
  readonly label: string;
  readonly language: string | undefined;
}) {
  const { t } = useTranslation();
  return (
    <div className="workspace-status">
      <span>{icon}</span>
      <div>
        <small>{label}</small>
        <strong>{t(`workspace.status.${health.status}`)}</strong>
        <time>
          {health.observedAtUnixMs === null
            ? t('workspace.diagnostic.neverObserved')
            : formatWorkspaceTime(health.observedAtUnixMs, language)}
        </time>
      </div>
      <StatusMark
        label={t(`workspace.status.${health.status}`)}
        pulse={health.status === 'connecting'}
        tone={workspaceStatusTone(health.status)}
      />
    </div>
  );
}
