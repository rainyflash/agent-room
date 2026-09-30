import { Banner, Button, Details, Spinner } from '@agent-room/ui-system';
import { Bot, LogOut, RefreshCw } from 'lucide-react';
import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { AppNavigation } from '@/shared/ui/app-navigation';
import type { AgentFleet } from '@/features/workspace/domain/agent-fleet';
import type { WorkspaceConnectionHealth } from '@/features/workspace/domain/connection-health';
import { AgentCardList } from '@/features/workspace/ui/agent-card-list';
import {
  AgentDetailsDialog,
  type AgentDeletionControl,
} from '@/features/workspace/ui/agent-details-dialog';
import { ConnectionStatusStrip } from '@/features/workspace/ui/connection-status-strip';
import { DeviceRail } from '@/features/workspace/ui/device-rail';
import { WorkspaceDiagnostics } from '@/features/workspace/ui/workspace-diagnostics';

export type AccountWorkspaceViewProps = {
  /** 登录的账户名，和“退出登录”放在一起看。 */
  readonly accountName: string;
  readonly connectionHealth: WorkspaceConnectionHealth;
  readonly failureCode: string | null;
  readonly fleet: AgentFleet;
  readonly loading: boolean;
  readonly onInvite: () => void;
  readonly onRefresh: () => void;
  readonly onSelectAgent: (agentId: string | null) => void;
  readonly onSignOut?: () => void;
  readonly selectedAgentId: string | null;
  /** 桌面端的“这台电脑”一节。 */
  readonly thisComputer?: ReactNode;
  /** 在别的电脑上开着的后台回复。 */
  readonly reception?: ReactNode;
  readonly agentDeletion?: AgentDeletionControl;
};

/**
 * 我的 Agent：一页看清有哪些 Agent、在哪儿、在不在线。主按钮是“接入 Agent”；设备和服务
 * 连接这类排查信息收在最下面的“详情”里。
 */
export function AccountWorkspaceView({
  accountName,
  connectionHealth,
  failureCode,
  fleet,
  loading,
  onInvite,
  onRefresh,
  onSelectAgent,
  onSignOut,
  selectedAgentId,
  thisComputer,
  reception,
  agentDeletion,
}: AccountWorkspaceViewProps) {
  const { t } = useTranslation();
  const [now] = useState(() => Date.now());
  const selected = fleet.agents.find((entry) => entry.agent.agentId === selectedAgentId) ?? null;

  return (
    <main className="account-workspace" id="main-content">
      <AppNavigation
        active="agents"
        actions={
          onSignOut === undefined ? undefined : (
            <Button
              aria-label={t('connection.action.logout')}
              className="app-navigation__sign-out"
              icon={<LogOut aria-hidden="true" />}
              onClick={onSignOut}
              size="compact"
              title={t('connection.action.logout')}
              tone="quiet"
            >
              {t('connection.action.logout')}
            </Button>
          )
        }
      />

      <header className="account-workspace__intro">
        <div>
          <h1>{t('workspace.title')}</h1>
          <p>{t('workspace.description')}</p>
          <p className="account-workspace__account">
            {t('workspace.signedInAs')} <strong>{accountName}</strong>
          </p>
        </div>
        <Button icon={<Bot aria-hidden="true" />} onClick={onInvite} size="large" tone="primary">
          {t('agentInvite.open')}
        </Button>
      </header>

      <section aria-labelledby="workspace-agents-title" className="account-workspace__agents">
        <h2 className="sr-only" id="workspace-agents-title">
          {t('workspace.list')}
        </h2>
        {failureCode !== null ? (
          <Banner
            action={
              <Button
                icon={<RefreshCw aria-hidden="true" />}
                onClick={onRefresh}
                size="compact"
                tone="alert"
              >
                {t('workspace.failed.retry')}
              </Button>
            }
            title={t('workspace.failed.title')}
            tone="danger"
          >
            <p>{t('workspace.failed.detail')}</p>
            <Details summary={t('workspace.details.more')}>
              <code>{failureCode}</code>
            </Details>
          </Banner>
        ) : loading ? (
          <p className="account-workspace__loading" role="status">
            <Spinner />
            {t('workspace.loading')}
          </p>
        ) : fleet.agents.length === 0 ? (
          <div className="account-workspace__empty">
            <Bot aria-hidden="true" />
            <h3>{t('workspace.empty.title')}</h3>
            <p>{t('workspace.empty.detail')}</p>
            <Button icon={<Bot aria-hidden="true" />} onClick={onInvite} tone="primary">
              {t('agentInvite.open')}
            </Button>
          </div>
        ) : (
          <AgentCardList agents={fleet.agents} now={now} onOpen={onSelectAgent} />
        )}
      </section>

      {thisComputer}
      {reception}

      <Details className="account-workspace__more" summary={t('workspace.more')}>
        <DeviceRail devices={fleet.devices} />
        <ConnectionStatusStrip health={connectionHealth} />
        <WorkspaceDiagnostics
          health={connectionHealth}
          orphanCount={fleet.orphanInstances.length}
        />
      </Details>

      {selected === null ? null : (
        <AgentDetailsDialog
          agent={selected}
          deletion={agentDeletion}
          now={now}
          onClose={() => {
            onSelectAgent(null);
          }}
        />
      )}
    </main>
  );
}
