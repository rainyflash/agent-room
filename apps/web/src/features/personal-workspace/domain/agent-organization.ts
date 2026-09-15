import type { LobbyAgent } from '@/features/lobby/domain/lobby';
import type { WorkspaceIndex } from './workspace-document';

export type AgentCollection = 'all' | 'favorites' | 'recent';

export function organizeAgents(
  agents: readonly LobbyAgent[],
  index: WorkspaceIndex | null,
  collection: AgentCollection,
  tag: string,
): readonly LobbyAgent[] {
  if (index === null) return agents;
  return agents
    .filter(
      (agent) =>
        (collection !== 'favorites' || index.favorites.has(agent.agentId)) &&
        (collection !== 'recent' || index.recent.has(agent.agentId)) &&
        (tag === '' || index.tags.get(agent.agentId)?.includes(tag) === true),
    )
    .toSorted((left, right) => {
      if (collection === 'recent') {
        const recent =
          (index.recent.get(right.agentId) ?? 0) - (index.recent.get(left.agentId) ?? 0);
        if (recent !== 0) return recent;
      }
      const favorite =
        Number(index.favorites.has(right.agentId)) - Number(index.favorites.has(left.agentId));
      return (
        favorite ||
        left.displayName.localeCompare(right.displayName) ||
        left.agentId.localeCompare(right.agentId)
      );
    });
}

export function projectTags(
  agents: readonly LobbyAgent[],
  index: WorkspaceIndex | null,
): readonly string[] {
  return index === null
    ? []
    : [...new Set(agents.flatMap((agent) => index.tags.get(agent.agentId) ?? []))].toSorted();
}

export function parseProjectTags(input: string): readonly string[] | null {
  const tags = [
    ...new Set(
      input
        .split(/[,，]/u)
        .map((tag) => tag.trim())
        .filter(Boolean),
    ),
  ];
  return tags.length > 12 || tags.some((tag) => tag.length > 32) ? null : tags;
}
