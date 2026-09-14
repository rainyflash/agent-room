import { describe, expect, it } from 'vitest';
import { BrowserConversationStorage, type SavedConversation } from './conversation-storage';

function memory() {
  const values = new Map<string, string>();
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
const draft: SavedConversation = {
  version: 1,
  text: '还没写完',
  mentions: ['@agent:server'],
  reply: null,
  pendingSubmissionId: null,
};
describe('durable conversation drafts', () => {
  it('restores only the same account and room, including reply identity', () => {
    const storage = memory();
    const a = new BrowserConversationStorage(storage, '@alice:server');
    const value = {
      ...draft,
      reply: {
        messageId: 'message',
        actor: { displayName: 'Agent', matrixUserId: '@agent:server' },
      },
    };
    expect(a.write('!room:server', value).ok).toBe(true);
    expect(new BrowserConversationStorage(storage, '@alice:server').read('!room:server')).toEqual({
      ok: true,
      value,
    });
    expect(a.read('!other:server')).toEqual({ ok: true, value: null });
    expect(new BrowserConversationStorage(storage, '@bob:server').read('!room:server')).toEqual({
      ok: true,
      value: null,
    });
  });
  it('reports storage denial instead of promising a saved draft', () => {
    const storage = {
      ...memory(),
      setItem: () => {
        throw new Error('quota');
      },
    };
    expect(new BrowserConversationStorage(storage, 'a').write('r', draft)).toEqual({
      ok: false,
      error: 'unavailable',
    });
  });
});
