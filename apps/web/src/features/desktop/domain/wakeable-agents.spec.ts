import { describe, expect, it } from 'vitest';

import type { ReceiverView } from './reception';
import type { ReceptionRecord } from './reception-ownership';
import { wakeableAgentIds } from './wakeable-agents';

const ROOM = '!room:matrix.test';
const NOW = 1_700_000_000_000;
const id = (suffix: number) => `0198b601-77a5-74f1-b4f4-940f2919${String(suffix).padStart(4, '0')}`;

function record(agent: number, overrides: Partial<ReceptionRecord> = {}): ReceptionRecord {
  return {
    agentId: id(agent),
    catalogId: id(9000),
    roomId: ROOM,
    sessionKey: id(9001),
    displayName: 'Scout',
    instanceId: id(9002),
    deviceId: id(9003),
    deviceLabel: 'Studio desktop',
    runId: id(9004),
    status: 'active',
    nextDeviceId: null,
    lastSeenUnixMs: NOW - 5_000,
    revision: 1,
    progress: { afterEventId: null, pending: null, retry: null },
    ...overrides,
  };
}

function receiver(
  agent: number | null,
  overrides: { readonly running?: boolean; readonly enabled?: boolean; readonly roomId?: string },
): ReceiverView {
  return {
    running: overrides.running ?? true,
    failure: null,
    progress: null,
    state: {
      binding: {
        session: { sessionKey: id(9001), displayName: 'Scout' },
        policy: { roomId: overrides.roomId ?? ROOM, allowedPrincipalId: id(9005) },
        automationGrantId: id(9006),
        host: { taskId: id(9007), executable: 'host', mcpExecutable: 'mcp', workspace: '.' },
        start: { mode: 'now' },
      },
      bridgeService: 'bridge',
      agentId: agent === null ? null : id(agent),
      roomCatalogId: id(9000),
      instanceId: id(9002),
      checkpoint: { state: 'ready', afterEventId: null },
      enabled: overrides.enabled ?? true,
      lastDelivery: null,
    },
  };
}

describe('wakeableAgentIds', () => {
  it('账户上记着、刚报过到的后台回复算叫得醒，哪台电脑上的都算', () => {
    expect([...wakeableAgentIds(ROOM, [record(1), record(2)], NOW, [])]).toEqual([id(1), id(2)]);
  });

  it('别的房间的、停了的、交接中的、很久没报到的都不算', () => {
    const records = [
      record(1, { roomId: '!other:matrix.test' }),
      record(2, { status: 'idle' }),
      record(3, { status: 'draining' }),
      record(4, { lastSeenUnixMs: NOW - 60_000 }),
    ];
    expect(wakeableAgentIds(ROOM, records, NOW, []).size).toBe(0);
  });

  it('按读到记录的那一刻算，不按现在：两次重读之间不会自己变成联系不上', () => {
    const records = [record(1, { lastSeenUnixMs: NOW - 5_000 })];
    // 记录是 NOW 读到的：那一刻它刚报过到。
    expect(wakeableAgentIds(ROOM, records, NOW, []).has(id(1))).toBe(true);
    // 拿一分钟以后的“现在”去比同一份记录，就成了很久没报到。
    expect(wakeableAgentIds(ROOM, records, NOW + 60_000, []).has(id(1))).toBe(false);
  });

  it('这台电脑上正在跑、开着的接收也算；停着的、关了的、别的房间的、还没认出 Agent 的不算', () => {
    const receivers = [
      receiver(1, {}),
      receiver(2, { running: false }),
      receiver(3, { enabled: false }),
      receiver(4, { roomId: '!other:matrix.test' }),
      receiver(null, {}),
    ];
    expect([...wakeableAgentIds(ROOM, [], NOW, receivers)]).toEqual([id(1)]);
  });

  it('两处都记着的只算一个', () => {
    expect(wakeableAgentIds(ROOM, [record(1)], NOW, [receiver(1, {})]).size).toBe(1);
  });
});
