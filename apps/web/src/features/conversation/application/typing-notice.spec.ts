import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { TypingNotice } from './typing-notice';

const roomId = '!lobby:agent-room.test';

describe('TypingNotice', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  function notice() {
    const typing = vi.fn<(roomId: string, typing: boolean) => void>();
    return { typing, notice: new TypingNotice({ typing }, roomId, () => Date.now()) };
  }

  it('开始打字说一次，一直在打就隔 20 秒续一次', () => {
    const { typing, notice: current } = notice();
    current.changed('你');
    current.changed('你好');
    expect(typing.mock.calls).toEqual([[roomId, true]]);

    for (let step = 0; step < 4; step += 1) {
      vi.advanceTimersByTime(5_000);
      current.changed(`你好${String(step)}`);
    }
    expect(typing.mock.calls).toEqual([
      [roomId, true],
      [roomId, true],
    ]);
  });

  it('停 10 秒、清空或发出去就说停了，只说一次', () => {
    const { typing, notice: current } = notice();
    current.changed('在吗');
    vi.advanceTimersByTime(9_999);
    expect(typing).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1);
    expect(typing).toHaveBeenLastCalledWith(roomId, false);

    current.changed('再问一句');
    current.changed('  ');
    expect(typing.mock.calls.slice(2)).toEqual([
      [roomId, true],
      [roomId, false],
    ]);

    current.stop();
    vi.advanceTimersByTime(20_000);
    expect(typing).toHaveBeenCalledTimes(4);
  });
});
