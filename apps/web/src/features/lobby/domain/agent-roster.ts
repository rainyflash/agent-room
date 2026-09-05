import type { LobbyAgent, LobbyAgentStatus } from './lobby';

export function filterLobbyAgents(
  agents: readonly LobbyAgent[],
  query: string,
  status: LobbyAgentStatus | 'all',
): readonly LobbyAgent[] {
  const normalized = query.trim().toLocaleLowerCase();
  return agents.filter(
    (agent) =>
      (status === 'all' || agent.status === status) &&
      (agent.displayName.toLocaleLowerCase().includes(normalized) ||
        agent.matrixUserId.toLocaleLowerCase().includes(normalized)),
  );
}
