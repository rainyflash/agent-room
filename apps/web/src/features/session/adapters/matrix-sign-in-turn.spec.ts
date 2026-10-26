import { describe, expect, it } from 'vitest';

import { MatrixSignInTurn } from './matrix-sign-in-turn';

const KEY = 'agent-room.matrix-sign-in.v1:https://matrix.agent-room.test';

/** 一个浏览器：共用的 localStorage，每个标签页各有一份 sessionStorage；时间由测试推着走。 */
function browser() {
  const shared = memoryStorage();
  let now = 1_000_000;
  const steps: (() => void)[] = [];
  const tab = (onPause?: (ms: number) => void) =>
    new MatrixSignInTurn({
      key: KEY,
      now: () => now,
      pause: (ms) => {
        now += ms;
        onPause?.(ms);
        steps.shift()?.();
        return Promise.resolve();
      },
      shared,
      tab: memoryStorage(),
    });
  return {
    advance: (ms: number) => {
      now += ms;
    },
    holder: () => shared.getItem(KEY),
    /** 下一次停顿时做的事（别的标签页正好在这时动手）。 */
    meanwhile: (step: () => void) => {
      steps.push(step);
    },
    now: () => now,
    shared,
    tab,
  };
}

describe('聊天登录一次只让一个标签页走', () => {
  it('没人在走就直接轮到自己，走完撤掉记号', async () => {
    const page = browser();
    const turn = page.tab();

    await expect(turn.take()).resolves.toEqual({ waited: false });
    expect(page.holder()).not.toBeNull();

    turn.release();
    expect(page.holder()).toBeNull();
  });

  it('别的标签页正在走：等它撤掉记号再轮到自己，并说等过', async () => {
    const page = browser();
    const first = page.tab();
    await first.take();
    let pauses = 0;
    const second = page.tab(() => {
      pauses += 1;
      if (pauses === 3) first.release();
    });

    await expect(second.take()).resolves.toEqual({ waited: true });
    expect(pauses).toBeGreaterThanOrEqual(3);
    // 现在记号是第二个的：第一个再撤也撤不掉别人的。
    first.release();
    expect(page.holder()).not.toBeNull();
    second.release();
    expect(page.holder()).toBeNull();
  });

  it('记号过了 30 秒不算数：关掉的标签页挡不住别人', async () => {
    const page = browser();
    await page.tab().take();
    page.advance(30_000);

    await expect(page.tab().take()).resolves.toEqual({ waited: false });
  });

  it('两个标签页同时记下时后写的算数，另一个接着等', async () => {
    const page = browser();
    const early = page.tab();
    // early 记下以后、回头再看之前，另一个标签页也记下了，这次算它的；它走完撤掉，early 才轮到。
    page.meanwhile(() => {
      page.shared.setItem(KEY, JSON.stringify({ at: page.now(), tab: 'other-tab' }));
    });
    page.meanwhile(() => {
      page.shared.removeItem(KEY);
    });

    await expect(early.take()).resolves.toEqual({ waited: true });
    expect(page.holder()).not.toContain('other-tab');
    early.release();
    expect(page.holder()).toBeNull();
  });

  it('同一个标签页跳去登录又回来，还认得自己的记号', async () => {
    const page = browser();
    const tabStorage = memoryStorage();
    const before = new MatrixSignInTurn({
      key: KEY,
      pause: () => Promise.resolve(),
      shared: page.shared,
      tab: tabStorage,
    });
    await before.take();

    const after = new MatrixSignInTurn({ key: KEY, shared: page.shared, tab: tabStorage });
    after.release();
    expect(page.holder()).toBeNull();
  });

  it('浏览器不让用 localStorage 时不排队', async () => {
    const throwing = {
      getItem: () => {
        throw new Error('denied');
      },
      removeItem: () => {
        throw new Error('denied');
      },
      setItem: () => {
        throw new Error('denied');
      },
    };
    const turn = new MatrixSignInTurn({ key: KEY, shared: throwing, tab: memoryStorage() });
    await expect(turn.take()).resolves.toEqual({ waited: false });
    expect(() => {
      turn.release();
    }).not.toThrow();

    const none = new MatrixSignInTurn({ key: KEY, shared: undefined, tab: memoryStorage() });
    await expect(none.take()).resolves.toEqual({ waited: false });
  });
});

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    clear: () => {
      values.clear();
    },
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    get length() {
      return values.size;
    },
    removeItem: (key) => {
      values.delete(key);
    },
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}
