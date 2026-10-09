import type { LobbyAgent } from './lobby';

/** 按名字或 Matrix ID 搜索，按名字排。在线与否的分组由名单自己排。 */
export function filterLobbyAgents(
  agents: readonly LobbyAgent[],
  query: string,
): readonly LobbyAgent[] {
  const normalized = query.trim().toLocaleLowerCase();
  return agents
    .filter(
      (agent) =>
        agent.displayName.toLocaleLowerCase().includes(normalized) ||
        agent.matrixUserId.toLocaleLowerCase().includes(normalized),
    )
    .toSorted((left, right) => left.displayName.localeCompare(right.displayName));
}
