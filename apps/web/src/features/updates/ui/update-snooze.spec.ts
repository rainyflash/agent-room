import { describe, expect, it } from 'vitest';

import {
  readUpdateSnooze,
  snoozeRemainingMs,
  updateSnoozeMs,
  writeUpdateSnooze,
} from './update-snooze';

function memoryStorage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    removeItem: (key: string) => {
      values.delete(key);
    },
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
    values,
  };
}

describe('更新提醒的“稍后”', () => {
  it('同一个版本 24 小时以后再提醒，更新的版本马上提醒', () => {
    const snooze = { version: '0.1.0-alpha.68', untilUnixMs: 1_000 + updateSnoozeMs };
    expect(snoozeRemainingMs(snooze, '0.1.0-alpha.68', 1_000)).toBe(updateSnoozeMs);
    expect(snoozeRemainingMs(snooze, '0.1.0-alpha.68', 1_000 + updateSnoozeMs)).toBe(0);
    expect(snoozeRemainingMs(snooze, '0.1.0-alpha.69', 1_000)).toBe(0);
    expect(snoozeRemainingMs(null, '0.1.0-alpha.68', 1_000)).toBe(0);
    // 时钟往回调过，也最多再等 24 小时。
    expect(snoozeRemainingMs(snooze, '0.1.0-alpha.68', 0)).toBe(updateSnoozeMs);
  });

  it('记在本地存储里，重开以后还在；坏的记录当作没点过', () => {
    const storage = memoryStorage();
    const snooze = { version: '0.1.0-alpha.68', untilUnixMs: 42 };
    writeUpdateSnooze(storage, snooze);
    expect(readUpdateSnooze(storage)).toEqual(snooze);
    writeUpdateSnooze(storage, null);
    expect(readUpdateSnooze(storage)).toBeNull();
    storage.values.set('agent-room.desktop-update.snoozed', '{not json');
    expect(readUpdateSnooze(storage)).toBeNull();
    storage.values.set('agent-room.desktop-update.snoozed', JSON.stringify({ version: '' }));
    expect(readUpdateSnooze(storage)).toBeNull();
    expect(readUpdateSnooze(null)).toBeNull();
  });

  it('本地存储读写出错时不影响提醒', () => {
    const broken = {
      getItem: () => {
        throw new Error('blocked');
      },
      removeItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
    };
    expect(readUpdateSnooze(broken)).toBeNull();
    expect(() => {
      writeUpdateSnooze(broken, { version: '0.1.0-alpha.68', untilUnixMs: 1 });
    }).not.toThrow();
  });
});
