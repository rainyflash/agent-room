import type { DesktopLobbySnapshot } from '@/features/desktop/domain/desktop-runtime';
import type { LobbyAgent, LobbyAgentStatus, LobbyRoom } from '@/features/lobby/domain/lobby';

export type DesktopLobbyProjection = {
  readonly messages: readonly DesktopLobbySnapshot['messages'][number][];
  readonly room: LobbyRoom;
};

const statusPriority: Readonly<Record<LobbyAgentStatus, number>> = Object.freeze({
  blocked: 5,
  waiting_input: 4,
  working: 3,
  completed: 2,
  idle: 1,
  offline: 0,
});

/** 把 Bridge 的实例级投影折叠为大厅的 Agent 级单一事实。 */
export function projectDesktopLobby(
  snapshot: DesktopLobbySnapshot,
  roomName: string,
  topic: string,
): DesktopLobbyProjection {
  const agents = new Map<string, LobbyAgent>();
  for (const presence of snapshot.agents) {
    const current = agents.get(presence.agent.agentId);
    const status = dominantStatus(current?.status ?? 'offline', presence.status);
    agents.set(
      presence.agent.agentId,
      Object.freeze({
        agentId: presence.agent.agentId,
        ...(presence.agent.avatarUrl === null ? {} : { avatarUrl: presence.agent.avatarUrl }),
        displayName: presence.agent.displayName,
        instanceIds: Object.freeze([
          ...(current?.instanceIds ?? []),
          ...(current?.instanceIds.includes(presence.instanceId) === true
            ? []
            : [presence.instanceId]),
        ]),
        matrixUserId: presence.agent.matrixUserId,
        status,
        statusExpiresAtUnixMs: Math.max(
          current?.statusExpiresAtUnixMs ?? 0,
          presence.leaseExpiresAtUnixMs,
        ),
        trust: 'verified',
        visibility: 'coarse',
      }),
    );
  }

  if (!agents.has(snapshot.identity.agent.agentId)) {
    const identity = snapshot.identity.agent;
    agents.set(
      identity.agentId,
      Object.freeze({
        agentId: identity.agentId,
        ...(identity.avatarUrl === null ? {} : { avatarUrl: identity.avatarUrl }),
        displayName: identity.displayName,
        instanceIds: Object.freeze([snapshot.identity.instanceId]),
        matrixUserId: identity.matrixUserId,
        status: snapshot.identity.connectionState === 'ready' ? 'idle' : 'offline',
        statusExpiresAtUnixMs: snapshot.observedAtUnixMs + 60_000,
        trust: 'verified',
        visibility: 'coarse',
      }),
    );
  }

  return Object.freeze({
    messages: Object.freeze([...snapshot.messages]),
    room: Object.freeze({
      agents: Object.freeze(
        [...agents.values()].toSorted((left, right) =>
          left.displayName.localeCompare(right.displayName),
        ),
      ),
      name: roomName,
      observedAtUnixMs: snapshot.observedAtUnixMs,
      roomId: snapshot.identity.roomId,
      ...(topic.length === 0 ? {} : { topic }),
    }),
  });
}

function dominantStatus(current: LobbyAgentStatus, candidate: LobbyAgentStatus): LobbyAgentStatus {
  return statusPriority[candidate] > statusPriority[current] ? candidate : current;
}
