import { Button, Details } from '@agent-room/ui-system';
import { Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { agentActivity } from '@/features/workspace/domain/agent-card';
import type { FleetAgent } from '@/features/workspace/domain/agent-fleet';
import { AgentCardList } from '@/features/workspace/ui/agent-card-list';

/**
 * 在线的一组摊开，不在线的收起来、只写有几个（维护者 2026-10-17 定）：以前的 Agent 攒多了，
 * 一页全摊开就找不到正在用的。正在连接、连接不稳的也算在线那一组，它们这会儿还连着。
 *
 * 有 30 天没上线的，不在线那一组上面写一行、给“一键删除”，不用先展开那一组（维护者 2026-11-01 定）。
 */
export function AgentGroups({
  agents,
  now,
  onOpen,
  roomCounts,
  staleCleanup,
}: {
  readonly agents: readonly FleetAgent[];
  readonly now: number;
  readonly onOpen: (agentId: string) => void;
  readonly roomCounts?: ReadonlyMap<string, number> | undefined;
  /** 30 天没上线、能删的有几个；没有就不写那一行。 */
  readonly staleCleanup?: { readonly count: number; readonly onOpen: () => void } | undefined;
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
      {staleCleanup === undefined || staleCleanup.count === 0 ? null : (
        <div className="agent-groups__stale">
          <p>{t('workspace.stale.summary', { count: staleCleanup.count })}</p>
          <Button
            icon={<Trash2 aria-hidden="true" />}
            onClick={staleCleanup.onOpen}
            size="compact"
            tone="ghost"
          >
            {t('workspace.stale.open')}
          </Button>
        </div>
      )}
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
