import { describe, expect, it } from 'vitest';

import { projectDesktopLobby } from '@/features/desktop/domain/desktop-lobby-projection';
import type { DesktopLobbySnapshot } from '@/features/desktop/domain/desktop-runtime';

describe('桌面大厅投影', () => {
  it('按 Agent 合并多个实例，并在同步尚未返回自身状态时保留真实会话身份', () => {
    const projected = projectDesktopLobby(snapshot(), 'Public lobby', 'Live state');

    expect(projected.room.roomId).toBe('!public:matrix.test');
    expect(projected.room.agents).toHaveLength(2);
    expect(projected.room.agents[0]).toMatchObject({
      agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
      displayName: 'My Agent',
      instanceIds: ['0198b601-77a4-7bb8-83eb-a8fe68c97e44'],
      status: 'idle',
    });
    expect(projected.room.agents[1]).toMatchObject({
      agentId: '0198b601-77a2-7bb8-83eb-a8fe68c97e44',
      instanceIds: ['0198b601-77a5-7bb8-83eb-a8fe68c97e44', '0198b601-77a6-7bb8-83eb-a8fe68c97e44'],
      status: 'blocked',
    });
  });
});

function snapshot(): DesktopLobbySnapshot {
  const peer = {
    agentId: '0198b601-77a2-7bb8-83eb-a8fe68c97e44',
    avatarUrl: null,
    displayName: 'Peer Agent',
    matrixUserId: '@peer:matrix.test',
  } as const;
  return {
    agents: [
      {
        agent: peer,
        instanceId: '0198b601-77a5-7bb8-83eb-a8fe68c97e44',
        leaseExpiresAtUnixMs: 2_000,
        observedAtUnixMs: 1_000,
        roomId: '!public:matrix.test',
        status: 'working',
      },
      {
        agent: peer,
        instanceId: '0198b601-77a6-7bb8-83eb-a8fe68c97e44',
        leaseExpiresAtUnixMs: 2_100,
        observedAtUnixMs: 1_100,
        roomId: '!public:matrix.test',
        status: 'blocked',
      },
    ],
    identity: {
      agent: {
        agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
        avatarUrl: null,
        displayName: 'My Agent',
        matrixUserId: '@mine:matrix.test',
      },
      connectionState: 'ready',
      grantedCapabilities: [],
      instanceId: '0198b601-77a4-7bb8-83eb-a8fe68c97e44',
      matrixDeviceId: 'DEVICE',
      roomId: '!public:matrix.test',
    },
    messages: [],
    nextCursor: null,
    observedAtUnixMs: 1_200,
  };
}
