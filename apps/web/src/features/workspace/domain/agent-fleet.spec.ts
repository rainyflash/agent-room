import { describe, expect, it } from 'vitest';

import type { AgentInstance, ProductDevice } from '@/features/security/domain/access-management';
import type { OwnedAgent } from '@/features/workspace/domain/agent-directory';
import { projectAgentFleet } from '@/features/workspace/domain/agent-fleet';

const AGENT_A_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const AGENT_B_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e45';
const ORPHAN_AGENT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const CURRENT_DEVICE_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e47';
const REMOTE_DEVICE_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e48';

describe('Agent 舰队投影', () => {
  it('按稳定 Agent 身份聚合多设备实例并标记当前设备', () => {
    const result = projectAgentFleet({
      agents: [agent(AGENT_A_ID, 'Build Agent'), agent(AGENT_B_ID, 'Quiet Agent')],
      currentMatrixDeviceId: 'WEB-CURRENT',
      devices: [
        device(CURRENT_DEVICE_ID, 'Studio', 'WEB-CURRENT'),
        device(REMOTE_DEVICE_ID, 'Laptop', 'WEB-REMOTE'),
      ],
      instances: [
        instance(AGENT_A_ID, CURRENT_DEVICE_ID, 'online', 3_000),
        instance(AGENT_A_ID, REMOTE_DEVICE_ID, 'degraded', 2_000),
      ],
    });

    expect(result.agents[0]?.agent.displayName).toBe('Build Agent');
    expect(result.agents[0]?.instances).toHaveLength(2);
    expect(result.agents[0]?.instances[0]?.currentDevice).toBe(true);
    expect(result.agents[0]?.status).toBe('online');
    expect(result.agents[0]?.lastSeenAtUnixMs).toBe(3_000);
    expect(result.agents[1]?.status).toBe('offline');
    expect(result.devices[0]).toMatchObject({ current: true, instanceCount: 1, label: 'Studio' });
  });

  it('不伪造目录中不存在的 Agent，并暴露孤儿实例用于诊断', () => {
    const result = projectAgentFleet({
      agents: [agent(AGENT_A_ID, 'Build Agent')],
      currentMatrixDeviceId: null,
      devices: [device(REMOTE_DEVICE_ID, 'Laptop', 'WEB-REMOTE')],
      instances: [instance(ORPHAN_AGENT_ID, REMOTE_DEVICE_ID, 'online', 4_000)],
    });

    expect(result.agents).toHaveLength(1);
    expect(result.agents[0]?.instances).toHaveLength(0);
    expect(result.orphanInstances.map((value) => value.agentId)).toEqual([ORPHAN_AGENT_ID]);
    expect(result.devices[0]?.instanceCount).toBe(0);
  });

  it('在线状态问得到时按它算：在线、离开都是在线，离线带上离线那一刻', () => {
    const waiting = agent(AGENT_A_ID, 'Waiting Agent');
    const gone = agent(AGENT_B_ID, 'Gone Agent');
    const result = projectAgentFleet({
      agents: [waiting, gone],
      currentMatrixDeviceId: null,
      devices: [device(REMOTE_DEVICE_ID, 'Laptop', 'WEB-REMOTE')],
      instances: [
        // 只用 MCP 等消息的从来不续连接上的状态，一直停在正在连接。
        instance(AGENT_A_ID, REMOTE_DEVICE_ID, 'connecting', 1_500),
        // 发过一次自动回复的，库里一直写着在线。
        instance(AGENT_B_ID, REMOTE_DEVICE_ID, 'online', 3_000),
      ],
      presence: new Map([
        [waiting.matrixUserId, { state: 'unavailable' }],
        [
          gone.matrixUserId,
          { lastActiveAtUnixMs: 8_000, offlineSeenAtUnixMs: 9_000, state: 'offline' },
        ],
      ]),
    });

    expect(result.agents.map((entry) => [entry.agent.displayName, entry.status])).toEqual([
      ['Waiting Agent', 'online'],
      ['Gone Agent', 'offline'],
    ]);
    expect(result.agents[0]?.instances[0]?.status).toBe('online');
    expect(result.agents[1]?.lastSeenAtUnixMs).toBe(9_000);
    expect(result.agents[1]?.instances[0]).toMatchObject({
      lastSeenAtUnixMs: 9_000,
      status: 'offline',
    });
  });

  it('问不到在线状态、连接全撤销了的照旧看连接；好几个连接时不改各个连接', () => {
    const quiet = agent(AGENT_A_ID, 'Quiet Agent');
    const retired = agent(AGENT_B_ID, 'Retired Agent');
    const revoked = {
      ...instance(AGENT_B_ID, REMOTE_DEVICE_ID, 'revoked', 2_000),
      revokedAtUnixMs: 2_500,
    };
    const result = projectAgentFleet({
      agents: [quiet, retired],
      currentMatrixDeviceId: null,
      devices: [
        device(CURRENT_DEVICE_ID, 'Studio', 'WEB-CURRENT'),
        device(REMOTE_DEVICE_ID, 'Laptop', 'WEB-REMOTE'),
      ],
      instances: [
        instance(AGENT_A_ID, CURRENT_DEVICE_ID, 'connecting', 1_500),
        instance(AGENT_A_ID, REMOTE_DEVICE_ID, 'offline', 2_000),
        revoked,
      ],
      presence: new Map([
        [quiet.matrixUserId, { state: 'online' }],
        [retired.matrixUserId, { state: 'online' }],
      ]),
    });

    expect(result.agents[0]?.status).toBe('online');
    expect(result.agents[0]?.instances.map((value) => value.status)).toEqual([
      'connecting',
      'offline',
    ]);
    expect(result.agents[1]?.status).toBe('revoked');

    const unknown = projectAgentFleet({
      agents: [quiet],
      currentMatrixDeviceId: null,
      devices: [],
      instances: [instance(AGENT_A_ID, CURRENT_DEVICE_ID, 'connecting', 1_500)],
      presence: new Map(),
    });
    expect(unknown.agents[0]?.status).toBe('connecting');
  });

  it('把已撤销设备排在可信设备之后', () => {
    const revoked = {
      ...device(CURRENT_DEVICE_ID, 'Old PC', 'WEB-OLD'),
      revokedAtUnixMs: 5_000,
      trustState: 'revoked' as const,
    };
    const result = projectAgentFleet({
      agents: [],
      currentMatrixDeviceId: null,
      devices: [revoked, device(REMOTE_DEVICE_ID, 'Laptop', 'WEB-REMOTE')],
      instances: [],
    });

    expect(result.devices.map((value) => value.label)).toEqual(['Laptop', 'Old PC']);
  });
});

function agent(agentId: string, displayName: string): OwnedAgent {
  return {
    agentId,
    avatarContentId: null,
    description: '',
    displayName,
    matrixUserId: `@_agent_${agentId.slice(-4)}:matrix.test`,
    registeredAtUnixMs: 1_000,
    slug: displayName.toLowerCase().replaceAll(' ', '-'),
    visibility: 'private',
  };
}

function device(deviceId: string, label: string, matrixDeviceId: string): ProductDevice {
  return {
    createdAtUnixMs: 1_000,
    deviceId,
    label,
    lastSeenAtUnixMs: 2_000,
    matrixDeviceId,
    platform: 'windows',
    revokedAtUnixMs: null,
    trustState: 'verified',
  };
}

function instance(
  agentId: string,
  deviceId: string,
  status: AgentInstance['status'],
  lastSeenAtUnixMs: number,
): AgentInstance {
  return {
    adapterType: 'codex',
    agentAvatarContentId: null,
    agentDisplayName: 'Agent',
    agentId,
    agentInstanceId: `${agentId.slice(0, -1)}${deviceId.slice(-1)}`,
    capabilityVersion: '1.0',
    createdAtUnixMs: 1_000,
    device: { deviceId, label: 'Device', platform: 'windows', trustState: 'verified' },
    lastSeenAtUnixMs,
    matrixDeviceId: `AR-${deviceId.slice(-4)}`,
    matrixDeviceRevokedAtUnixMs: null,
    revokedAtUnixMs: null,
    status,
  };
}
