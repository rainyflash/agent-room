import { Details } from '@agent-room/ui-system';
import { useTranslation } from 'react-i18next';

import { agentActivity } from '@/features/workspace/domain/agent-card';
import type { FleetAgent } from '@/features/workspace/domain/agent-fleet';
import { AgentCardList } from '@/features/workspace/ui/agent-card-list';

/**
 * 在线的一组摊开，不在线的收起来、只写有几个（维护者 2026-10-17 定）：以前的 Agent 攒多了，
 * 一页全摊开就找不到正在用的。正在连接、连接不稳的也算在线那一组，它们这会儿还连着。
 */
export function AgentGroups({
  agents,
  now,
  onOpen,
  roomCounts,
}: {
  readonly agents: readonly FleetAgent[];
  readonly now: number;
  readonly onOpen: (agentId: string) => void;
  readonly roomCounts?: ReadonlyMap<string, number> | undefined;
}) {
  const { t } = useTranslation();
  const online = agents.filter(isOnline);
  const offline = agents.filter((entry) => !isOnline(entry));
  return (
    <div className="agent-groups">
      <section aria-labelledby="workspace-agents-online" className="agent-group">
        <h3 className="agent-group__title" id="workspace-agents-online">
          {t('workspace.group.online', { count: online.length })}
        </h3>
        {online.length === 0 ? (
          <p className="agent-group__empty">{t('workspace.group.noneOnline')}</p>
        ) : (
          <AgentCardList agents={online} now={now} onOpen={onOpen} roomCounts={roomCounts} />
        )}
      </section>
      {offline.length === 0 ? null : (
        <Details
          className="agent-groups__offline"
          summary={t('workspace.group.offline', { count: offline.length })}
        >
          <AgentCardList agents={offline} now={now} onOpen={onOpen} roomCounts={roomCounts} />
        </Details>
      )}
    </div>
  );
}

function isOnline(entry: FleetAgent): boolean {
  const { kind } = agentActivity(entry);
  return kind === 'online' || kind === 'connecting' || kind === 'degraded';
}
