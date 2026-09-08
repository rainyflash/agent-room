import { describe, expect, it } from 'vitest';
import type { LobbyAgent, LobbyRoom } from './lobby';
import { roomCrowdGroups } from './room-crowd';
import { isWalkableFloor } from './room-floor';
import {
  projectLobbyScene,
  type LobbySceneProjection,
  type LobbyViewport,
} from './scene-projection';

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

  it.each([200, 1000])('%i 人总览准确统计全部成员，桌面与手机的组数都有界', (count) => {
    const scene = projectLobbyScene(room(count), null);
    for (const width of [1440, 390]) {
      const viewport = fit(scene, width, 800);
      const groups = roomCrowdGroups(scene, viewport);
      expect(groups.length).toBeGreaterThan(0);
      expect(groups.length).toBeLessThanOrEqual(width < 768 ? 12 : 48);
      expect(groups.reduce((sum, group) => sum + group.count, 0)).toBe(count);
      expect(groups.reduce((sum, group) => sum + group.working, 0)).toBe(count / 4);
      expect(groups.reduce((sum, group) => sum + group.attention, 0)).toBe(count / 2);
      for (const [index, group] of groups.entries()) {
        for (const other of groups.slice(index + 1))
          expect(Math.hypot(group.x - other.x, group.y - other.y) * viewport.zoom).toBeGreaterThan(
            80,
          );
      }
    }
  });

  it('平移只裁剪组，不改变组内人数；放大和小房间均显示具体人物', () => {
    const scene = projectLobbyScene(room(1000), null);
    const viewport = fit(scene, 1440, 800);
    const full = roomCrowdGroups(scene, viewport);
    const panned = roomCrowdGroups(scene, { ...viewport, x: scene.world.width / 2 });
    expect(panned.length).toBeLessThan(full.length);
    expect(panned.length).toBeGreaterThan(0);
    for (const group of panned)
      expect(full.find((candidate) => candidate.id === group.id)).toEqual(group);
    expect(roomCrowdGroups(scene, { ...viewport, zoom: 0.85 })).toEqual([]);
    expect(roomCrowdGroups(projectLobbyScene(room(24), null), viewport)).toEqual([]);
  });
});

function fit(scene: LobbySceneProjection, width: number, height: number): LobbyViewport {
  const zoom = Math.min((width - 96) / scene.world.width, (height - 96) / scene.world.height);
  return { x: -48 / zoom, y: -48 / zoom, width: width / zoom, height: height / zoom, zoom };
}

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
