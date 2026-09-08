import { describe, expect, it } from 'vitest';
import type { LobbyAgent, LobbyRoom } from './lobby';
import { roomHome, roomMapDestination, roomMapViewport } from './room-map';
import { isWalkableFloor } from './room-floor';
import { projectLobbyScene } from './scene-projection';

describe('大房间容量与分层浏览', () => {
  it('1,000 位在场 Agent 都有独立可行走位置，人物间距不随人数缩小', () => {
    const scene = projectLobbyScene(room(1000), null);
    expect(scene.nodes).toHaveLength(1000);
    expect(new Set(scene.nodes.map((node) => `${String(node.x)}:${String(node.y)}`)).size).toBe(
      1000,
    );
    const floor = { width: scene.world.width, depth: scene.world.height };
    expect(scene.nodes.every((node) => isWalkableFloor(node, 18, floor))).toBe(true);
    let nearest = Number.POSITIVE_INFINITY;
    for (let index = 0; index < scene.nodes.length; index += 1) {
      const node = scene.nodes[index];
      if (node === undefined) throw new Error('Missing projected member.');
      for (const other of scene.nodes.slice(index + 1))
        nearest = Math.min(nearest, Math.hypot(node.x - other.x, node.y - other.y));
    }
    expect(nearest).toBeGreaterThanOrEqual(128);
  });

  it('从 200 人扩展到 1,000 人以及有人离场时，留场成员位置稳定', () => {
    const initial = projectLobbyScene(room(200), null);
    const expanded = projectLobbyScene(room(1000), null, { previous: initial.layout });
    const departed = projectLobbyScene(room(200), null, { previous: expanded.layout });
    expect(expanded.world.width * expanded.world.height).toBeGreaterThan(
      initial.world.width * initial.world.height,
    );
    for (const [id, position] of initial.layout) {
      expect(expanded.layout.get(id)).toEqual(position);
      expect(departed.layout.get(id)).toEqual(position);
    }
  });

  it('状态和输入顺序不构成空间分组，首次布局与已有布局均保持相同位置', () => {
    const initialRoom = room(200);
    const initial = projectLobbyScene(initialRoom, null);
    const changedRoom = {
      ...initialRoom,
      agents: initialRoom.agents
        .toReversed()
        .map((agent): LobbyAgent => ({ ...agent, status: 'idle' })),
    };
    const fresh = projectLobbyScene(changedRoom, null);
    const updated = projectLobbyScene(changedRoom, null, { previous: initial.layout });
    expect(updated.world).toEqual(initial.world);
    for (const [id, position] of initial.layout) {
      expect(fresh.layout.get(id)).toEqual(position);
      expect(updated.layout.get(id)).toEqual(position);
    }
  });

  it('初始镜头靠近本人，自己与所有 Agent 仍有独立位置', () => {
    const scene = projectLobbyScene(room(1000), null, {
      humans: [{ matrixUserId: '@me:test', displayName: 'Me', isSelf: true }],
    });
    const self = scene.humans?.[0];
    if (self === undefined) throw new Error('Missing human.');
    expect(self.y).toBeGreaterThan(scene.world.height - 500);
    expect(roomHome(scene)).toEqual({ x: self.x, y: self.y });
    expect(scene.nodes.every((node) => Math.hypot(node.x - self.x, node.y - self.y) >= 128)).toBe(
      true,
    );
    const withoutSelf = projectLobbyScene(room(1000), null);
    expect(roomHome(withoutSelf)).toEqual({
      x: withoutSelf.world.width / 2,
      y: withoutSelf.world.height / 2,
    });
  });

  it('地图点击映射到真实房间坐标，并约束越界或非有限输入', () => {
    const world = { width: 10000, height: 7000 };
    expect(roomMapDestination(world, 0.75, 0.25)).toEqual({ x: 7500, y: 1750 });
    expect(roomMapDestination(world, -1, 2)).toEqual({ x: 0, y: 7000 });
    expect(roomMapDestination(world, Number.NaN, Number.POSITIVE_INFINITY)).toEqual({
      x: 5000,
      y: 3500,
    });
  });

  it('小地图视口仅显示镜头与房间交集，边缘人物定位不会让框溢出', () => {
    const world = { width: 1000, height: 700 };
    expect(roomMapViewport(world, { x: -100, y: 500, width: 600, height: 400, zoom: 1 })).toEqual({
      x: 0,
      y: 500,
      width: 500,
      height: 200,
    });
    expect(roomMapViewport(world, { x: 1100, y: 800, width: 600, height: 400, zoom: 1 })).toEqual({
      x: 1000,
      y: 700,
      width: 0,
      height: 0,
    });
  });
});

function room(count: number): LobbyRoom {
  const statuses = ['working', 'idle', 'waiting_input', 'blocked'] as const;
  return {
    roomId: '!capacity:test',
    name: 'Capacity room',
    observedAtUnixMs: 1_700_000_000_000,
    agents: Array.from({ length: count }, (_, index): LobbyAgent => ({
      agentId: `agent-${String(index)}`,
      displayName: `Agent ${String(index)}`,
      instanceIds: [`instance-${String(index)}`],
      matrixUserId: `@agent-${String(index)}:test`,
      status: statuses[index % statuses.length] ?? 'idle',
      statusExpiresAtUnixMs: 1_700_000_030_000,
      trust: 'unknown',
      visibility: 'coarse',
    })),
  };
}
