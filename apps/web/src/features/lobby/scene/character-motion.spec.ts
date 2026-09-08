import { sceneCharacters } from './scene-character';
import { describe, expect, it } from 'vitest';
import type { LobbyAgent, LobbyAgentStatus } from '../domain/lobby';
import {
  isWalkableFloor,
  nearbyWalkableFloor,
  projectFloorPoint,
  roomFurnishings,
  unprojectFloorPoint,
} from '../domain/room-floor';
import { projectLobbyScene } from '../domain/scene-projection';
import { characterPose } from './character-motion';

const statuses: readonly LobbyAgentStatus[] = [
  'idle',
  'working',
  'completed',
  'waiting_input',
  'blocked',
  'offline',
];
const agents: readonly LobbyAgent[] = Array.from({ length: 200 }, (_, index) => ({
  agentId: `agent-${String(index)}`,
  displayName: `角色 ${String(index)}`,
  instanceIds: [`instance-${String(index)}`],
  matrixUserId: `@agent-${String(index)}:test`,
  status: statuses[index % statuses.length] ?? 'idle',
  statusExpiresAtUnixMs: 300_000,
  lastPolledAtUnixMs: 0,
  trust: 'unknown',
  visibility: 'coarse',
}));
const scene = projectLobbyScene(
  { agents, name: '房间', observedAtUnixMs: 0, roomId: '!room:test' },
  null,
);

describe('角色活动与家具边界', () => {
  it('未接待和已完成的人物不持续假装执行任务', () => {
    for (const node of sceneCharacters(scene)) {
      const unknown = { ...node, reception: 'unknown' as const };
      expect(characterPose(unknown, 4, true).moving).toBe(false);
      expect(characterPose({ ...node, status: 'completed' }, 4, true).moving).toBe(false);
    }
  });
  it('所有角色出生在可行走的地面，家具内部会寻找安全位置', () => {
    for (const node of sceneCharacters(scene)) {
      expect(node.floorPosition).toBeDefined();
      expect(isWalkableFloor(node.floorPosition ?? { x: 0, y: 0 }, 18, node.floor)).toBe(true);
    }
    for (const furniture of roomFurnishings) {
      const center = { x: furniture.x + furniture.width / 2, y: furniture.y + furniture.depth / 2 };
      expect(isWalkableFloor(center)).toBe(false);
      expect(isWalkableFloor(nearbyWalkableFloor(center))).toBe(true);
    }
    expect(new Set(scene.nodes.map((node) => node.radius)).size).toBe(1);
  });

  it('走动覆盖完整循环且不会穿过家具或走出房间', () => {
    let walking = 0;
    const roomy = projectLobbyScene(
      {
        agents: agents.slice(0, 6).map((agent) => ({ ...agent, status: 'idle' })),
        name: '工作室',
        roomId: '!studio:test',
        observedAtUnixMs: 0,
      },
      null,
    );
    for (const node of [...sceneCharacters(scene), ...sceneCharacters(roomy)]) {
      for (let time = 0; time < 36; time += 0.5) {
        const pose = characterPose(node, time, true);
        expect(isWalkableFloor(unprojectFloorPoint(pose), 17.9, node.floor)).toBe(true);
        if (pose.moving) walking += 1;
      }
    }
    expect(walking).toBeGreaterThan(100);
  });

  it('离线和减少动画状态保持静止，不改写业务投影', () => {
    const before = structuredClone(scene);
    for (const node of sceneCharacters(scene)) {
      const still = { x: node.x, y: node.y, stride: 0, facing: 1, moving: false };
      expect(characterPose(node, 20, false)).toEqual(still);
      if (node.status === 'offline') expect(characterPose(node, 7, true)).toEqual(still);
      expect(characterPose(node, 11, true)).toEqual(characterPose(node, 11, true));
      expect(projectFloorPoint(node.floorPosition ?? { x: 0, y: 0 })).toEqual({
        x: node.x,
        y: node.y,
      });
    }
    expect(scene).toEqual(before);
  });
});
