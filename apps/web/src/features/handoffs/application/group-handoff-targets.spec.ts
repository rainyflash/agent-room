import { describe, expect, it } from 'vitest';

import { groupHandoffTargets } from '@/features/handoffs/application/group-handoff-targets';
import { handoffTargetFixture } from '@/features/handoffs/testing/handoff-fixtures';

describe('交接目标分组', () => {
  it('按 Agent 和设备聚合，并把在线设备与实例排在前面', () => {
    const offline = {
      ...handoffTargetFixture,
      device: {
        ...handoffTargetFixture.device,
        deviceId: '01990d9e-8400-7000-8000-000000000023',
        label: 'Travel PC',
      },
      instanceId: '01990d9e-8400-7000-8000-000000000022',
      instanceStatus: 'offline' as const,
      online: false,
    };
    const secondOnline = {
      ...handoffTargetFixture,
      adapterType: 'claude-desktop',
      instanceId: '01990d9e-8400-7000-8000-000000000032',
    };

    const groups = groupHandoffTargets([offline, secondOnline, handoffTargetFixture]);

    expect(groups).toHaveLength(1);
    expect(groups[0]?.devices.map(({ device }) => device.label)).toEqual([
      'Studio PC',
      'Travel PC',
    ]);
    expect(groups[0]?.devices[0]?.targets.map(({ adapterType }) => adapterType)).toEqual([
      'claude-desktop',
      'codex-desktop',
    ]);
  });

  it('不会把名称相同但稳定 ID 不同的 Agent 合并', () => {
    const secondAgent = {
      ...handoffTargetFixture,
      agentId: '01990d9e-8400-7000-8000-000000000041',
      device: {
        ...handoffTargetFixture.device,
        deviceId: '01990d9e-8400-7000-8000-000000000043',
      },
      instanceId: '01990d9e-8400-7000-8000-000000000042',
    };

    expect(groupHandoffTargets([handoffTargetFixture, secondAgent])).toHaveLength(2);
  });
});
