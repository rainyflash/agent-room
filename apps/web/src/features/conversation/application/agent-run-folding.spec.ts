import { describe, expect, it } from 'vitest';

import { agentRunFoldingEnabled, rememberAgentRunFolding } from './agent-run-folding';

function memoryStorage(initial: Readonly<Record<string, string>> = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
    removeItem: (key: string) => {
      values.delete(key);
    },
  };
}

describe('Agent 之间的长串折不折', () => {
  it('没选过的设备是折的', () => {
    expect(agentRunFoldingEnabled(memoryStorage())).toBe(true);
    expect(agentRunFoldingEnabled(null)).toBe(true);
  });

  it('关掉、再打开，下次读得回来', () => {
    const storage = memoryStorage();
    rememberAgentRunFolding(false, storage);
    expect(agentRunFoldingEnabled(storage)).toBe(false);
    rememberAgentRunFolding(true, storage);
    expect(agentRunFoldingEnabled(storage)).toBe(true);
    // 打开时不留值：以后默认换了，没动过的设备跟着走。
    expect(storage.getItem('agent-room.conversation.agent-run-folding')).toBeNull();
  });

  it('不认识的值、读写不了的存储都当成折', () => {
    expect(
      agentRunFoldingEnabled(
        memoryStorage({ 'agent-room.conversation.agent-run-folding': 'sometimes' }),
      ),
    ).toBe(true);
    const broken = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
      removeItem: () => {
        throw new Error('blocked');
      },
    };
    expect(agentRunFoldingEnabled(broken)).toBe(true);
    expect(() => {
      rememberAgentRunFolding(false, broken);
    }).not.toThrow();
  });
});
