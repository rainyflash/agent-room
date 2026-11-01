import { describe, expect, it } from 'vitest';

import type { AgentInstance } from '@/features/security/domain/access-management';
import type { OwnedAgent } from '@/features/workspace/domain/agent-directory';
import { projectAgentFleet, type FleetAgent } from '@/features/workspace/domain/agent-fleet';
import {
  STALE_AGENT_AFTER_MS,
  staleAgentCandidates,
  staleAgents,
} from '@/features/workspace/domain/stale-agents';

const DAY = 24 * 60 * 60 * 1_000;
const NOW = 100 * DAY;
const OLD_AGENT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e41';
const OLDER_AGENT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const RECENT_AGENT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
const DEVICE_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e47';

describe('30 天没上线的 Agent', () => {
  it('设备 30 天没用的算，最久没上线的排前面', () => {
    const agents = fleet(
      [agent(OLD_AGENT_ID, 'Old'), agent(OLDER_AGENT_ID, 'Older')],
      [instance(OLD_AGENT_ID, 'a'), instance(OLDER_AGENT_ID, 'b')],
    );
    const result = staleAgents({
      activity: new Map([
        [instanceId(OLD_AGENT_ID, 'a'), NOW - 40 * DAY],
        [instanceId(OLDER_AGENT_ID, 'b'), NOW - 60 * DAY],
      ]),
      agents,
      canDelete: () => true,
      now: NOW,
    });

    expect(result.map((stale) => stale.entry.agent.displayName)).toEqual(['Older', 'Old']);
    expect(result[0]?.lastOnlineAtUnixMs).toBe(NOW - 60 * DAY);
  });

  it('设备最近用过的不算，哪怕连接上记的时间很早', () => {
    const result = staleAgents({
      activity: new Map([[instanceId(RECENT_AGENT_ID, 'a'), NOW - 2 * DAY]]),
      agents: fleet([agent(RECENT_AGENT_ID, 'Recent')], [instance(RECENT_AGENT_ID, 'a')]),
      canDelete: () => true,
      now: NOW,
    });

    expect(result).toEqual([]);
  });

  it('还在用的连接有一个没问到，就先不算', () => {
    const result = staleAgents({
      activity: new Map([[instanceId(OLD_AGENT_ID, 'a'), NOW - 40 * DAY]]),
      agents: fleet(
        [agent(OLD_AGENT_ID, 'Old')],
        [instance(OLD_AGENT_ID, 'a'), instance(OLD_AGENT_ID, 'b')],
      ),
      canDelete: () => true,
      now: NOW,
    });

    expect(result).toEqual([]);
  });

  it('还没问到设备时一个也不给', () => {
    const result = staleAgents({
      activity: null,
      agents: fleet([agent(OLD_AGENT_ID, 'Old')], [instance(OLD_AGENT_ID, 'a')]),
      canDelete: () => true,
      now: NOW,
    });

    expect(result).toEqual([]);
  });

  it('设备不在了的看连接和注册的时间；刚注册的不算', () => {
    const agents = fleet(
      [agent(OLD_AGENT_ID, 'Old'), agent(RECENT_AGENT_ID, 'Recent', NOW - 5 * DAY)],
      [instance(OLD_AGENT_ID, 'a'), instance(RECENT_AGENT_ID, 'b')],
    );
    const result = staleAgents({
      activity: new Map([
        [instanceId(OLD_AGENT_ID, 'a'), null],
        [instanceId(RECENT_AGENT_ID, 'b'), null],
      ]),
      agents,
      canDelete: () => true,
      now: NOW,
    });

    expect(result.map((stale) => stale.entry.agent.displayName)).toEqual(['Old']);
  });

  it('正在线的、网络 Agent、不让删的都不算', () => {
    const agents = fleet(
      [
        agent(OLD_AGENT_ID, 'Online'),
        agent(OLDER_AGENT_ID, 'Network'),
        agent(RECENT_AGENT_ID, 'Me'),
      ],
      [
        { ...instance(OLD_AGENT_ID, 'a'), status: 'online' },
        { ...instance(OLDER_AGENT_ID, 'b'), adapterType: 'network' },
        instance(RECENT_AGENT_ID, 'c'),
      ],
    );
    const result = staleAgents({
      activity: new Map([
        [instanceId(OLD_AGENT_ID, 'a'), NOW - 40 * DAY],
        [instanceId(OLDER_AGENT_ID, 'b'), NOW - 40 * DAY],
        [instanceId(RECENT_AGENT_ID, 'c'), NOW - 40 * DAY],
      ]),
      agents,
      canDelete: (entry) => entry.agent.agentId !== RECENT_AGENT_ID,
      now: NOW,
    });

    expect(result).toEqual([]);
  });

  it('连接全撤销了的按连接上最后的时间算', () => {
    const result = staleAgents({
      activity: new Map(),
      agents: fleet(
        [agent(OLD_AGENT_ID, 'Old')],
        [{ ...instance(OLD_AGENT_ID, 'a'), revokedAtUnixMs: NOW - 50 * DAY, status: 'revoked' }],
      ),
      canDelete: () => true,
      now: NOW,
    });

    expect(result.map((stale) => stale.lastOnlineAtUnixMs)).toEqual([NOW - 70 * DAY]);
  });

  it('不看设备时已经不够 30 天的，不用再去问设备', () => {
    const agents = fleet(
      [agent(OLD_AGENT_ID, 'Old'), agent(RECENT_AGENT_ID, 'Recent', NOW - 5 * DAY)],
      [instance(OLD_AGENT_ID, 'a'), instance(RECENT_AGENT_ID, 'b')],
    );

    expect(
      staleAgentCandidates({ agents, canDelete: () => true, now: NOW }).map(
        (stale) => stale.entry.agent.displayName,
      ),
    ).toEqual(['Old']);
  });

  it('正好 30 天的算', () => {
    const result = staleAgents({
      activity: new Map([[instanceId(OLD_AGENT_ID, 'a'), NOW - STALE_AGENT_AFTER_MS]]),
      agents: fleet([agent(OLD_AGENT_ID, 'Old')], [instance(OLD_AGENT_ID, 'a')]),
      canDelete: () => true,
      now: NOW,
    });

    expect(result).toHaveLength(1);
  });
});

function fleet(
  agents: readonly OwnedAgent[],
  instances: readonly AgentInstance[],
): readonly FleetAgent[] {
  return projectAgentFleet({ agents, currentMatrixDeviceId: null, devices: [], instances }).agents;
}

function agent(agentId: string, displayName: string, registeredAtUnixMs = 1_000): OwnedAgent {
  return {
    agentId,
    avatarContentId: null,
    description: '',
    displayName,
    matrixUserId: `@_agent_${agentId.slice(-4)}:matrix.test`,
    registeredAtUnixMs,
    slug: displayName.toLowerCase(),
    visibility: 'private',
  };
}

function instanceId(agentId: string, suffix: string): string {
  return `${agentId.slice(0, -1)}${suffix}`;
}

function instance(agentId: string, suffix: string): AgentInstance {
  return {
    adapterType: 'codex',
    agentAvatarContentId: null,
    agentDisplayName: 'Agent',
    agentId,
    agentInstanceId: instanceId(agentId, suffix),
    capabilityVersion: '1.0',
    createdAtUnixMs: NOW - 90 * DAY,
    device: { deviceId: DEVICE_ID, label: 'Device', platform: 'windows', trustState: 'verified' },
    lastSeenAtUnixMs: NOW - 70 * DAY,
    matrixDeviceId: `AR-${suffix}`,
    matrixDeviceRevokedAtUnixMs: null,
    revokedAtUnixMs: null,
    status: 'offline',
  };
}
