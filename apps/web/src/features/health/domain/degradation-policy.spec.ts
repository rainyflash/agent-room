import { describe, expect, it } from 'vitest';

import {
  resolveDegradedCapabilities,
  type ProductCapability,
  type RuntimeDependency,
} from './degradation-policy';

function capability(unavailable: readonly RuntimeDependency[], name: ProductCapability) {
  return resolveDegradedCapabilities(new Set(unavailable)).find(
    (decision) => decision.capability === name,
  );
}

describe('运行依赖降级策略', () => {
  it.each([
    ['control_plane', 'browse_lobby', 'read_only'],
    ['control_plane', 'join_room', 'blocked'],
    ['matrix', 'send_message', 'blocked'],
    ['object_storage', 'open_content', 'blocked'],
    ['oidc', 'authenticate', 'blocked'],
    ['bridge', 'agent_tools', 'blocked'],
    ['pixi', 'visual_lobby', 'read_only'],
  ] as const)('%s 故障时把 %s 明确降级为 %s', (dependency, name, status) => {
    expect(capability([dependency], name)).toEqual({
      capability: name,
      reasons: [dependency],
      status,
    });
  });

  it('Pixi 故障只关闭可视化，功能完整列表仍可浏览', () => {
    expect(capability(['pixi'], 'browse_lobby')).toEqual({
      capability: 'browse_lobby',
      reasons: [],
      status: 'available',
    });
  });

  it('多依赖故障采用最严格能力状态且保留决定性原因', () => {
    expect(capability(['control_plane', 'matrix'], 'browse_lobby')).toEqual({
      capability: 'browse_lobby',
      reasons: ['control_plane', 'matrix'],
      status: 'read_only',
    });
    expect(capability(['matrix', 'object_storage'], 'send_message')).toEqual({
      capability: 'send_message',
      reasons: ['matrix', 'object_storage'],
      status: 'blocked',
    });
  });
});
