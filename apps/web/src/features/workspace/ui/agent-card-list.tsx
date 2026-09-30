import type { TFunction } from 'i18next';
import { ChevronRight } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { AgentPortrait } from '@/features/lobby/ui/room-illustration';
import {
  agentActivity,
  agentPlace,
  type AgentActivity,
  type AgentPlace,
} from '@/features/workspace/domain/agent-card';
import type { FleetAgent } from '@/features/workspace/domain/agent-fleet';
import { formatRelativeTime } from '@/features/workspace/ui/workspace-format';

/** 每个 Agent 一张卡：头像、名字、在不在线、在哪儿运行。点开看详情。 */
export function AgentCardList({
  agents,
  now,
  onOpen,
}: {
  readonly agents: readonly FleetAgent[];
  readonly now: number;
  readonly onOpen: (agentId: string) => void;
}) {
  const { i18n, t } = useTranslation();
  return (
    <ul className="agent-cards">
      {agents.map((entry) => {
        const activity = agentActivity(entry);
        const place = agentPlace(entry);
        return (
          <li key={entry.agent.agentId}>
            <button
              className="agent-card"
              data-activity={activity.kind}
              onClick={() => {
                onOpen(entry.agent.agentId);
              }}
              type="button"
            >
              <span aria-hidden="true" className="agent-card__portrait">
                <AgentPortrait id={entry.agent.agentId} />
              </span>
              <span className="agent-card__body">
                <strong>{entry.agent.displayName}</strong>
                <span className="agent-card__activity">
                  <span aria-hidden="true" className="agent-card__dot" />
                  {activityText(t, activity, now, i18n.resolvedLanguage)}
                </span>
                {place.kind === 'none' ? null : (
                  <span className="agent-card__place">{placeText(t, place)}</span>
                )}
              </span>
              <ChevronRight aria-hidden="true" className="agent-card__chevron" />
            </button>
          </li>
        );
      })}
    </ul>
  );
}

export function activityText(
  t: TFunction,
  activity: AgentActivity,
  now: number,
  language: string | undefined,
): string {
  switch (activity.kind) {
    case 'online':
      return t('workspace.activity.online');
    case 'connecting':
      return t('workspace.activity.connecting');
    case 'degraded':
      return t('workspace.activity.degraded');
    case 'lastSeen':
      return t('workspace.activity.lastSeen', {
        time: formatRelativeTime(activity.atUnixMs, now, language),
      });
    case 'never':
      return t('workspace.activity.never');
  }
}

function placeText(t: TFunction, place: Exclude<AgentPlace, { readonly kind: 'none' }>): string {
  switch (place.kind) {
    case 'network':
      return t('workspace.place.network');
    case 'thisComputer':
      return t('workspace.place.thisComputer');
    case 'device':
      return t('workspace.place.device', { device: place.label });
    case 'devices':
      return t('workspace.place.devices', { count: place.count });
  }
}
