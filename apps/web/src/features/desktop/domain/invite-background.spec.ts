import { describe, expect, it } from 'vitest';

import type { HostSessionDiagnostics } from './desktop-runtime';
import {
  offersInviteBackground,
  readInviteBackground,
  saveInviteBackground,
} from './invite-background';

const catalogId = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const room = { roomId: '!room:test', catalogId };

function session(
  offer: Partial<NonNullable<HostSessionDiagnostics['receptionOffer']>> | null,
  extra: Partial<HostSessionDiagnostics['session']> = {},
): HostSessionDiagnostics {
  return {
    displayName: 'Scout',
    sessionKey: 'task',
    session: {
      sessionId: '0198b601-77a1-7bb8-83eb-a8fe68c97e48',
      state: 'ready',
      agentId: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
      errorCode: null,
      ...extra,
    },
    receptionOffer:
      offer === null
        ? null
        : {
            task: { taskId: 'task', hostType: 'codex', workspace: 'C:\\project' },
            roomId: '!room:test',
            roomCatalogId: catalogId,
            instanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e48',
            ...offer,
          },
    lastInboxReadAgoMs: 0,
    lastMessageReceivedAgoMs: null,
    lastMessageSentAgoMs: null,
  };
}

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
  } as unknown as Storage;
}

function blockedStorage(): Storage {
  const blocked = () => {
    throw new Error('blocked');
  };
  return { getItem: blocked, setItem: blocked } as unknown as Storage;
}

describe('接入对话框里的后台回复', () => {
  it('登记的是这间房的后台回复，才替人打开', () => {
    expect(offersInviteBackground(session({}), room)).toBe(true);
    // 同一个房间换了分片：按目录认。
    expect(offersInviteBackground(session({ roomId: '!shard:test' }), room)).toBe(true);
    expect(
      offersInviteBackground(
        session({ roomId: '!other:test', roomCatalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e99' }),
        room,
      ),
    ).toBe(false);
    // 从“我的 Agent”打开、没有房间时，它登记在哪间就开哪间。
    expect(offersInviteBackground(session({ roomId: '!other:test' }), null)).toBe(true);
  });

  it('还没登记、不在目录里的房间、还没准备好的连接：不开', () => {
    expect(offersInviteBackground(session(null), room)).toBe(false);
    expect(offersInviteBackground(session({ roomCatalogId: null }), room)).toBe(false);
    expect(offersInviteBackground(session({}, { state: 'starting' }), room)).toBe(false);
    expect(offersInviteBackground(session({}, { agentId: null }), room)).toBe(false);
  });

  it('默认勾上，人关掉过就记住；存不了时照默认', () => {
    const storage = memoryStorage();
    expect(readInviteBackground(storage)).toBe(true);
    saveInviteBackground(storage, false);
    expect(readInviteBackground(storage)).toBe(false);
    saveInviteBackground(storage, true);
    expect(readInviteBackground(storage)).toBe(true);

    expect(readInviteBackground(blockedStorage())).toBe(true);
    expect(() => {
      saveInviteBackground(blockedStorage(), false);
    }).not.toThrow();
    expect(readInviteBackground(null)).toBe(true);
  });
});
