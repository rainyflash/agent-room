import type { LobbyAgentNodeProjection } from './scene-projection';

export type SpatialDirection = 'down' | 'left' | 'right' | 'up';

export function nextAgentInDirection(
  nodes: readonly LobbyAgentNodeProjection[],
  currentAgentId: string | null,
  direction: SpatialDirection,
): string | null {
  if (nodes.length === 0) {
    return null;
  }
  const current = nodes.find((node) => node.agentId === currentAgentId) ?? nodes[0];
  if (current === undefined) {
    return null;
  }
  const vector = directionVector(direction);
  let winner: LobbyAgentNodeProjection | null = null;
  let winnerScore = Number.POSITIVE_INFINITY;

  for (const candidate of nodes) {
    if (candidate.agentId === current.agentId) {
      continue;
    }
    const deltaX = candidate.x - current.x;
    const deltaY = candidate.y - current.y;
    const forward = deltaX * vector.x + deltaY * vector.y;
    if (forward <= 0) {
      continue;
    }
    const lateral = Math.abs(deltaX * vector.y - deltaY * vector.x);
    const score = forward + lateral * 2.4;
    if (
      score < winnerScore ||
      (score === winnerScore && winner !== null && candidate.agentId < winner.agentId)
    ) {
      winner = candidate;
      winnerScore = score;
    }
  }
  return winner?.agentId ?? current.agentId;
}

function directionVector(direction: SpatialDirection): { readonly x: number; readonly y: number } {
  const vectors: Readonly<Record<SpatialDirection, { readonly x: number; readonly y: number }>> = {
    down: { x: 0, y: 1 },
    left: { x: -1, y: 0 },
    right: { x: 1, y: 0 },
    up: { x: 0, y: -1 },
  };
  return vectors[direction];
}
