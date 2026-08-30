import { Button } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { CircleAlert, LoaderCircle, RefreshCw, ShieldCheck } from 'lucide-react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { LanguageControl } from '@/features/preferences/ui/language-control';
import type { AgentFleet, FleetAgent } from '@/features/workspace/domain/agent-fleet';
import type { WorkspaceConnectionHealth } from '@/features/workspace/domain/connection-health';
import { AgentFleetList } from '@/features/workspace/ui/agent-fleet-list';
import { AgentInspector } from '@/features/workspace/ui/agent-inspector';
import { ConnectionStatusStrip } from '@/features/workspace/ui/connection-status-strip';
import { DeviceRail } from '@/features/workspace/ui/device-rail';
import { WorkspaceDiagnostics } from '@/features/workspace/ui/workspace-diagnostics';

export type AccountWorkspaceViewProps = {
  readonly connectionHealth: WorkspaceConnectionHealth;
  readonly failureCode: string | null;
  readonly fleet: AgentFleet;
  readonly loading: boolean;
  readonly onRefresh: () => void;
  readonly onSelectAgent: (agentId: string) => void;
  readonly principalDisplayName: string;
  readonly selectedAgentId: string | null;
};

export function AccountWorkspaceView({
  connectionHealth,
  failureCode,
  fleet,
  loading,
  onRefresh,
  onSelectAgent,
  principalDisplayName,
  selectedAgentId,
}: AccountWorkspaceViewProps) {
  const { t } = useTranslation();
  const selected = selectedFleetAgent(fleet, selectedAgentId);

  return (
    <main className="account-workspace" id="main-content">
      <header className="account-workspace__topbar">
        <a aria-label={t('app.name')} className="account-workspace__brand" href="/">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </a>
        <div className="account-workspace__topbar-actions">
          <LanguageControl />
          <Link params={{ section: 'security' }} to="/settings/$section">
            <ShieldCheck aria-hidden="true" />
            <span>{t('workspace.security')}</span>
          </Link>
        </div>
      </header>

      <section className="account-workspace__intro">
        <div>
          <p className="eyebrow">{t('workspace.eyebrow')}</p>
          <h1>{t('workspace.title')}</h1>
          <p>{t('workspace.description')}</p>
        </div>
        <dl>
          <Metric label={t('workspace.account')} value={principalDisplayName} />
          <Metric label={t('workspace.agents')} value={String(fleet.agents.length)} />
          <Metric label={t('workspace.devices')} value={String(fleet.devices.length)} />
          <Metric label={t('workspace.instances')} value={String(instanceCount(fleet))} />
        </dl>
      </section>

      <ConnectionStatusStrip health={connectionHealth} />
      <WorkspaceDiagnostics health={connectionHealth} orphanCount={fleet.orphanInstances.length} />

      {failureCode === null ? null : (
        <WorkspaceBoundary
          action={
            <Button icon={<RefreshCw aria-hidden="true" />} onClick={onRefresh} tone="alert">
              {t('workspace.failed.retry')}
            </Button>
          }
          detail={t('workspace.failed.detail')}
          icon={<CircleAlert aria-hidden="true" />}
          role="alert"
          title={t('workspace.failed.title')}
        >
          <code>{failureCode}</code>
        </WorkspaceBoundary>
      )}

      {failureCode === null && loading ? (
        <WorkspaceBoundary
          detail={t('workspace.description')}
          icon={<LoaderCircle aria-hidden="true" className="workspace-spin" />}
          role="status"
          title={t('workspace.loading')}
        />
      ) : null}

      {failureCode === null && !loading ? (
        <section className="account-workspace__body">
          <DeviceRail devices={fleet.devices} />
          <AgentFleetList
            agents={fleet.agents}
            onRefresh={onRefresh}
            onSelectAgent={onSelectAgent}
            selectedAgentId={selected?.agent.agentId ?? null}
          />
          <AgentInspector agent={selected} />
        </section>
      ) : null}
    </main>
  );
}

function WorkspaceBoundary({
  action,
  children,
  detail,
  icon,
  role,
  title,
}: {
  readonly action?: ReactNode;
  readonly children?: ReactNode;
  readonly detail: string;
  readonly icon: ReactNode;
  readonly role: 'alert' | 'status';
  readonly title: string;
}) {
  return (
    <section className="workspace-boundary" role={role}>
      {icon}
      <div>
        <h2>{title}</h2>
        <p>{detail}</p>
        {children}
      </div>
      {action}
    </section>
  );
}

function Metric({ label, value }: { readonly label: string; readonly value: string }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

function selectedFleetAgent(fleet: AgentFleet, requestedId: string | null): FleetAgent | null {
  if (fleet.agents.length === 0) return null;
  return (
    fleet.agents.find((entry) => entry.agent.agentId === requestedId) ?? fleet.agents[0] ?? null
  );
}

function instanceCount(fleet: AgentFleet): number {
  return fleet.agents.reduce((total, entry) => total + entry.instances.length, 0);
}
