import { describe, expect, it } from 'vitest';

import type { LobbyAgent, LobbyAgentStatus, LobbyRoom } from './lobby';
import { projectLobbyScene, sceneDetailForZoom, visibleLobbyNodes } from './scene-projection';
import { nextAgentInDirection } from './spatial-navigation';

describe('大厅场景投影', () => {
  it('输入顺序变化时仍生成完全相同的确定性布局', () => {
    const agents = Array.from({ length: 200 }, (_, index) => agent(index));
    const first = projectLobbyScene(room(agents), null);
    const second = projectLobbyScene(room(agents.toReversed()), null);

    expect(second).toEqual(first);
    expect(first.nodes).toHaveLength(200);
    expect(new Set(first.nodes.map((node) => [node.x, node.y].join(':'))).size).toBe(200);
    expect(Object.isFrozen(first)).toBe(true);
    expect(Object.isFrozen(first.nodes)).toBe(true);
    expect(first.nodes.every((node) => Object.isFrozen(node))).toBe(true);
  });

  it('状态决定主题区且失效选中项不会泄漏到投影', () => {
    const projection = projectLobbyScene(
      room([
        agent(1, 'working'),
        agent(2, 'blocked'),
        agent(3, 'waiting_input'),
        agent(4, 'idle'),
        agent(5, 'offline'),
      ]),
      'missing-agent',
    );

    expect(projection.selectedAgentId).toBeNull();
    expect(Object.fromEntries(projection.nodes.map((node) => [node.status, node.zoneId]))).toEqual({
      blocked: 'attention',
      idle: 'available',
      offline: 'available',
      waiting_input: 'attention',
      working: 'active',
    });
  });

  it('按视口裁剪节点并按缩放只返回三个闭合层级', () => {
    const projection = projectLobbyScene(
      room(Array.from({ length: 60 }, (_, index) => agent(index))),
      null,
    );
    const visible = visibleLobbyNodes(projection, {
      height: 300,
      width: 400,
      x: 0,
      y: 0,
      zoom: 1,
    });

    expect(visible.length).toBeGreaterThan(0);
    expect(visible.length).toBeLessThan(projection.nodes.length);
    expect(sceneDetailForZoom(Number.NaN)).toBe('distant');
    expect(sceneDetailForZoom(0.67)).toBe('distant');
    expect(sceneDetailForZoom(0.68)).toBe('medium');
    expect(sceneDetailForZoom(1.18)).toBe('near');
  });

  it('方向导航优先前进方向并在边界稳定停留', () => {
    const base = agent(0);
    const nodes = [
      { ...base, agentId: 'center', x: 100, y: 100 },
      { ...base, agentId: 'right-near', x: 180, y: 110 },
      { ...base, agentId: 'right-off-axis', x: 150, y: 260 },
      { ...base, agentId: 'left', x: 20, y: 100 },
    ].map((node) => ({ ...node, radius: 24, zoneId: 'active' as const }));

    expect(nextAgentInDirection(nodes, 'center', 'right')).toBe('right-near');
    expect(nextAgentInDirection(nodes, 'center', 'left')).toBe('left');
    expect(nextAgentInDirection(nodes, 'right-near', 'right')).toBe('right-near');
    expect(nextAgentInDirection([], null, 'right')).toBeNull();
  });
});

function room(agents: readonly LobbyAgent[]): LobbyRoom {
  return {
    agents,
    name: '真实大厅',
    observedAtUnixMs: 1_700_000_000_000,
    roomId: '!lobby:agent-room.test',
  };
}

function agent(index: number, status: LobbyAgentStatus = statusAt(index)): LobbyAgent {
  const suffix = String(index).padStart(3, '0');
  return {
    agentId: `agent-${suffix}`,
    displayName: `Agent ${suffix}`,
    instanceIds: [`instance-${suffix}`],
    matrixUserId: `@agent-${suffix}:agent-room.test`,
    status,
    statusExpiresAtUnixMs: 1_700_000_030_000,
    trust: 'unknown',
    visibility: 'coarse',
  };
}

function statusAt(index: number): LobbyAgentStatus {
  const statuses: readonly LobbyAgentStatus[] = [
    'offline',
    'idle',
    'working',
    'waiting_input',
    'blocked',
    'completed',
  ];
  return statuses[index % statuses.length] ?? 'offline';
}
