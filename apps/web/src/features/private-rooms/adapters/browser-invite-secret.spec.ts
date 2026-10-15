import { afterEach, describe, expect, it, vi } from 'vitest';

import { forgetInviteSecret, readInviteSecret, stashInviteSecret } from './browser-invite-secret';

const SECRET = '0123456789abcdefghjkmnpqrs';

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    clear: () => {
      values.clear();
    },
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => {
      values.delete(key);
    },
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}

function history() {
  return { replaceState: vi.fn(), state: { idx: 3 } };
}

afterEach(() => {
  forgetInviteSecret(null);
});

describe('邀请链接的秘密', () => {
  it('打开 /join#秘密 时收进这个标签页，再从地址栏去掉', () => {
    const storage = memoryStorage();
    const navigation = history();

    stashInviteSecret(
      { hash: `#${SECRET.toUpperCase()}`, pathname: '/join', search: '?from=chat' },
      navigation,
      storage,
    );

    expect(navigation.replaceState).toHaveBeenCalledWith({ idx: 3 }, '', '/join?from=chat');
    expect(readInviteSecret(storage)).toBe(SECRET);
    forgetInviteSecret(storage);
    expect(readInviteSecret(storage)).toBeNull();
  });

  it('秘密不像样的也从地址栏去掉，并忘掉之前收着的', () => {
    const storage = memoryStorage();
    stashInviteSecret({ hash: `#${SECRET}`, pathname: '/join', search: '' }, history(), storage);
    const navigation = history();

    stashInviteSecret(
      { hash: '#not-a-secret', pathname: '/join', search: '' },
      navigation,
      storage,
    );

    expect(navigation.replaceState).toHaveBeenCalledWith({ idx: 3 }, '', '/join');
    expect(readInviteSecret(storage)).toBeNull();
  });

  it('别的页面、没有 # 的 /join 不动地址栏', () => {
    const storage = memoryStorage();
    const navigation = history();
    stashInviteSecret({ hash: `#${SECRET}`, pathname: '/rooms', search: '' }, navigation, storage);
    stashInviteSecret({ hash: '', pathname: '/join', search: '' }, navigation, storage);
    expect(navigation.replaceState).not.toHaveBeenCalled();
    expect(readInviteSecret(storage)).toBeNull();
  });

  it('存不进 sessionStorage 时这一页里照样用得上', () => {
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
    stashInviteSecret({ hash: `#${SECRET}`, pathname: '/join', search: '' }, history(), broken);
    expect(readInviteSecret(broken)).toBe(SECRET);
  });
});
