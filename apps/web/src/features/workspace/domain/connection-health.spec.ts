import { describe, expect, it } from 'vitest';

import type { BridgePhase } from '@/features/desktop/domain/desktop-runtime';
import type { AgentFleet } from '@/features/workspace/domain/agent-fleet';
import {
  bridgeWorkspaceStatus,
  projectWorkspaceConnectionHealth,
} from '@/features/workspace/domain/connection-health';

describe('工作区四层连接状态', () => {
  it('不会把部分失败的 Control Plane 探针伪装成在线', () => {
    const health = projectWorkspaceConnectionHealth({
      agents: { failureCode: null, fleet: emptyFleet, loading: false },
      bridge: {
        available: false,
        changedAtUnixMs: null,
        failureCode: null,
        phase: undefined,
      },
      controlPlane: {
        failureCode: 'control.devices_failed',
        observedAtUnixMs: 2_000,
        pending: false,
        results: [{ ok: true }, { ok: false }, { ok: true }],
      },
      matrix: {
        failureCode: null,
        observedAtUnixMs: 3_000,
        pending: false,
        result: { ok: true },
      },
    });

    expect(health.controlPlane.status).toBe('degraded');
    expect(health.matrix.status).toBe('online');
    expect(health.bridge.status).toBe('unavailable');
    expect(health.agents.status).toBe('offline');
  });

  it('Agent 混合在线与降级实例时汇总为降级而不是笼统在线', () => {
    const health = projectWorkspaceConnectionHealth({
      agents: {
        failureCode: null,
        fleet: fleetWithStatuses(['online', 'degraded']),
        loading: false,
      },
      bridge: {
        available: true,
        changedAtUnixMs: 4_000,
        failureCode: null,
        phase: 'ready',
      },
      controlPlane: {
        failureCode: null,
        observedAtUnixMs: 2_000,
        pending: false,
        results: [{ ok: true }],
      },
      matrix: {
        failureCode: 'matrix.offline',
        observedAtUnixMs: 3_000,
        pending: false,
        result: { ok: false },
      },
    });

    expect(health.agents.status).toBe('degraded');
    expect(health.bridge.status).toBe('online');
    expect(health.matrix.status).toBe('offline');
    expect(health.agents.observedAtUnixMs).toBe(1_001);
  });

  it('完整映射 Bridge 生命周期且未知阶段无法进入类型系统', () => {
    const cases: readonly [boolean, BridgePhase | undefined, string][] = [
      [false, undefined, 'unavailable'],
      [true, 'starting', 'connecting'],
      [true, 'ready', 'online'],
      [true, 'retry_scheduled', 'degraded'],
      [true, 'stopped', 'offline'],
    ];
    for (const [available, phase, expected] of cases) {
      expect(bridgeWorkspaceStatus(available, phase)).toBe(expected);
    }
  });
});

const emptyFleet: AgentFleet = Object.freeze({ agents: [], devices: [], orphanInstances: [] });

function fleetWithStatuses(statuses: readonly ('degraded' | 'online')[]): AgentFleet {
  return {
    agents: [
      {
        agent: {
          agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
          avatarContentId: null,
          description: '',
          displayName: 'Build Agent',
          matrixUserId: '@build:agent-room.test',
          registeredAtUnixMs: 1,
          slug: 'build-agent',
          visibility: 'private',
        },
        instances: statuses.map((status, index) => {
          const position = String(index + 1);
          const suffix = String(index + 5);
          return {
            adapterType: 'codex',
            agentAvatarContentId: null,
            agentDisplayName: 'Build Agent',
            agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
            agentInstanceId: `0198b601-77a1-7bb8-83eb-a8fe68c97e5${suffix}`,
            capabilityVersion: '1',
            createdAtUnixMs: 1_000 + index,
            currentDevice: index === 0,
            device: {
              deviceId: `0198b601-77a1-7bb8-83eb-a8fe68c97e4${suffix}`,
              label: `Device ${position}`,
              platform: 'windows',
              trustState: 'verified',
            },
            lastSeenAtUnixMs: 1_000 + index,
            matrixDeviceId: `AR-${position}`,
            matrixDeviceRevokedAtUnixMs: null,
            revokedAtUnixMs: null,
            status,
          };
        }),
        lastSeenAtUnixMs: 1_001,
        status: 'online',
      },
    ],
    devices: [],
    orphanInstances: [],
  };
}
