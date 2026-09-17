import { describe, expect, it } from 'vitest';

import {
  partitionAutomationGrants,
  type AutomationGrant,
} from '@/features/automation/domain/automation-grant';

const NOW = 1_760_000_000_000;

describe('partitionAutomationGrants', () => {
  it('把已撤销、已耗尽和已过期的授权移出当前列表', () => {
    const live = grant('01', { expiresAtUnixMs: NOW + 60_000 });
    const revoked = grant('02', { revokedAtUnixMs: NOW - 10_000, status: 'revoked' });
    const exhausted = grant('03', { status: 'exhausted' });
    const expired = grant('04', { status: 'expired' });

    const { active, history } = partitionAutomationGrants([expired, live, exhausted, revoked], NOW);

    expect(active.map((entry) => entry.grantId)).toEqual([live.grantId]);
    expect(history.map((entry) => entry.grantId)).toEqual([
      expired.grantId,
      exhausted.grantId,
      revoked.grantId,
    ]);
  });

  it('授权窗口已过但状态尚未同步时也算历史', () => {
    const stale = grant('05', { expiresAtUnixMs: NOW - 1 });

    const { active, history } = partitionAutomationGrants([stale], NOW);

    expect(active).toEqual([]);
    expect(history.map((entry) => entry.grantId)).toEqual([stale.grantId]);
  });

  it('没有授权时两侧都为空', () => {
    expect(partitionAutomationGrants([], NOW)).toEqual({ active: [], history: [] });
  });
});

function grant(suffix: string, overrides: Partial<AutomationGrant>): AutomationGrant {
  return {
    agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
    agentInstanceId: null,
    audience: 'known_room_members',
    expiresAtUnixMs: NOW + 30_000,
    grantId: `0198b601-77a1-7bb8-83eb-a8fe68c97e${suffix}`,
    maxMessagesPerMinute: 6,
    maxTotalMessages: null,
    messageKinds: ['reply'],
    messagesInCurrentMinute: 0,
    requiresRiskScan: true,
    revokedAtUnixMs: null,
    roomCatalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
    startsAtUnixMs: NOW - 60_000,
    status: 'active',
    totalMessages: 0,
    ...overrides,
  };
}
