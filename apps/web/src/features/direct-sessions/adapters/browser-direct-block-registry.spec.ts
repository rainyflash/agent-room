import { describe, expect, it } from 'vitest';

import { BrowserDirectBlockRegistry } from './browser-direct-block-registry';

const AGENT_ID = '0198b601-77a1-7bb8-83eb-a8fe68c97e52';

describe('BrowserDirectBlockRegistry', () => {
  it('本地屏蔽跨实例恢复且拒绝污染的存储载荷', () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => {
        values.set(key, value);
      },
    };
    const registry = new BrowserDirectBlockRegistry(storage);

    registry.set(AGENT_ID, true);
    expect(new BrowserDirectBlockRegistry(storage).has(AGENT_ID)).toBe(true);

    values.set('agent-room.direct-blocks.v1', JSON.stringify(['not-an-agent-id']));
    expect(new BrowserDirectBlockRegistry(storage).has(AGENT_ID)).toBe(false);
  });

  it('浏览器存储拒绝写入时仍保持进程内立即屏蔽', () => {
    const registry = new BrowserDirectBlockRegistry({
      getItem: () => null,
      setItem: () => {
        throw new Error('quota');
      },
    });

    registry.set(AGENT_ID, true);

    expect(registry.has(AGENT_ID)).toBe(true);
  });
});
