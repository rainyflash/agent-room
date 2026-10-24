import { describe, expect, it } from 'vitest';
import {
  AUTOMATIC_RESTART_WINDOW_MS,
  authenticationCallbackFailure,
  claimAutomaticRestart,
} from './authentication-callback';

function memoryStorage(): Pick<Storage, 'getItem' | 'setItem'> {
  const values = new Map<string, string>();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}

describe('authenticationCallbackFailure', () => {
  it('只认连接页上的两种原因', () => {
    expect(authenticationCallbackFailure('/connect?authentication=failed')).toBe('failed');
    expect(authenticationCallbackFailure('/connect?authentication=expired#x')).toBe('expired');
    expect(authenticationCallbackFailure('/connect?authentication=other')).toBeNull();
    expect(authenticationCallbackFailure('/rooms?authentication=failed')).toBeNull();
  });
});

describe('claimAutomaticRestart', () => {
  it('五分钟里只自动重来一次，过了再给一次', () => {
    const storage = memoryStorage();
    const start = 1_800_000_000_000;
    expect(claimAutomaticRestart(storage, start)).toBe(true);
    expect(claimAutomaticRestart(storage, start + 1_000)).toBe(false);
    expect(claimAutomaticRestart(storage, start + AUTOMATIC_RESTART_WINDOW_MS - 1)).toBe(false);
    expect(claimAutomaticRestart(storage, start + AUTOMATIC_RESTART_WINDOW_MS)).toBe(true);
  });

  it('记号比现在还晚（电脑时间被往回调过）时不当回事', () => {
    const storage = memoryStorage();
    expect(claimAutomaticRestart(storage, 2_000_000_000_000)).toBe(true);
    expect(claimAutomaticRestart(storage, 1_900_000_000_000)).toBe(true);
  });

  it('存储读写不了就不自动重来', () => {
    const broken: Pick<Storage, 'getItem' | 'setItem'> = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
    };
    expect(claimAutomaticRestart(broken, 1_800_000_000_000)).toBe(false);
  });
});
