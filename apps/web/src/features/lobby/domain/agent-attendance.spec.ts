import { describe, expect, it } from 'vitest';
import type { LobbyAgent } from './lobby';
import { agentAttendance, agentAvatarState, agentReception } from './agent-attendance';
import { projectLobbyScene } from './scene-projection';
import { projectFloorPoint, unprojectFloorPoint } from './room-floor';

const agent: LobbyAgent = {
  agentId: 'agent',
  displayName: 'Mira',
  instanceIds: ['instance'],
  matrixUserId: '@mira:studio.test',
  status: 'idle',
  statusExpiresAtUnixMs: 100_000,
  trust: 'unknown',
  visibility: 'coarse',
};

describe('工作室接待与在场规则', () => {
  it('后台在线与任务完成都不能证明接待，旧宿主保持未知', () => {
    expect(agentReception(agent, 50_000)).toBe('unknown');
    expect(agentAttendance({ ...agent, status: 'completed' }, 50_000)).toBe('present');
    expect(agentReception({ ...agent, status: 'completed' }, 50_000)).toBe('unknown');
  });
  it('实际等待证据会过期，最近读取不能冒充持续等待', () => {
    const polled = { ...agent, lastPolledAtUnixMs: 10_000, listeningUntilUnixMs: null };
    expect(agentReception(polled, 20_000)).toBe('waiting');
    expect(agentReception({ ...polled, listeningUntilUnixMs: 25_000 }, 20_000)).toBe('recent');
    expect(agentReception({ ...polled, listeningUntilUnixMs: 25_000 }, 25_000)).toBe('waiting');
    expect(agentReception(polled, 45_000)).toBe('waiting');
    expect(agentReception({ ...agent, lastPolledAtUnixMs: 90_000 }, 50_000)).toBe('unknown');
  });
  it('显式离线立即退场，租约过期有 30 秒重连宽限', () => {
    expect(agentAttendance({ ...agent, status: 'offline' }, 90_000)).toBe('away');
    const expired = { ...agent, status: 'offline' as const, reportedStatus: 'idle' as const };
    expect(agentAttendance(expired, 100_000)).toBe('reconnecting');
    expect(agentAttendance(expired, 129_999)).toBe('reconnecting');
    expect(agentAttendance(expired, 130_000)).toBe('away');
    expect(agentAttendance(agent, 130_000)).toBe('away');
  });
  it('头像只分三种：在等消息、离线、别的', () => {
    const waiting = { ...agent, lastPolledAtUnixMs: 10_000, listeningUntilUnixMs: 25_000 };
    expect(agentAvatarState(waiting, 20_000)).toBe('waiting');
    // 连着、没在等（读取方式未确认、下次运行时读）和重连中，都不画点。
    expect(agentAvatarState(agent, 50_000)).toBe('idle');
    expect(agentAvatarState({ ...waiting, listeningUntilUnixMs: null }, 20_000)).toBe('idle');
    expect(agentAvatarState(waiting, 25_000)).toBe('idle');
    const expired = { ...agent, status: 'offline' as const, reportedStatus: 'idle' as const };
    expect(agentAvatarState(expired, 100_000)).toBe('idle');
    expect(agentAvatarState(expired, 130_000)).toBe('offline');
    expect(agentAvatarState({ ...agent, status: 'offline' }, 90_000)).toBe('offline');
  });
  it('退场只改变空间投影，人物资料仍留在房间成员中', () => {
    const room = {
      agents: [{ ...agent, status: 'offline' as const }],
      name: 'studio',
      roomId: '!studio:test',
      observedAtUnixMs: 110_000,
    };
    const scene = projectLobbyScene(room, agent.agentId);
    expect(scene.nodes).toEqual([]);
    expect(scene.selectedAgentId).toBeNull();
    expect(room.agents[0]?.displayName).toBe('Mira');
    const returned = projectLobbyScene(
      { ...room, agents: [{ ...agent, statusExpiresAtUnixMs: 200_000 }] },
      null,
      { previous: scene.layout },
    );
    expect(returned.nodes.map((node) => node.agentId)).toEqual(['agent']);
  });
  it('地板四角和内部位置的反投影保持一致', () => {
    for (const x of [0, 300, 850, 1700])
      for (const y of [0, 350, 700, 1000]) {
        const point = unprojectFloorPoint(projectFloorPoint({ x, y }));
        expect(point.x).toBeCloseTo(x, 5);
        expect(point.y).toBeCloseTo(y, 5);
      }
  });
});
