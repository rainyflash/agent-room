import { describe, expect, it } from 'vitest';

import { WebObserverHandoffGateway } from './web-observer-handoff-gateway';

describe('纯 Web 上下文交付边界', () => {
  it('没有私有 Bridge IPC 时明确失败且不伪造目标实例', async () => {
    const gateway = new WebObserverHandoffGateway();

    const targets = await gateway.listTargets('!builders:agent-room.test');

    expect(targets).toEqual({
      error: { code: 'handoff.bridge_unavailable', retryable: false },
      ok: false,
    });
  });
});
