import { describe, expect, it } from 'vitest';

import {
  defaultInviteHost,
  inviteIdentityStorageKey,
  isCanonicalUuidV7,
  normalizeInviteDisplayName,
  projectInviteStatus,
  readInviteIdentity,
  writeInviteIdentity,
} from './agent-invite';
import type { HostSessionDiagnostics } from './desktop-runtime';

const key = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const otherKey = '0198b601-77a1-7bb8-83eb-a8fe68c97e45';

function session(overrides: Partial<HostSessionDiagnostics> = {}): HostSessionDiagnostics {
  return {
    displayName: 'Scout',
    session: {
      sessionId: '0198b601-77a1-7bb8-83eb-a8fe68c97e50',
      state: 'ready',
      agentId: null,
      errorCode: null,
    },
    sessionKey: key,
    lastInboxReadAgoMs: 1_000,
    lastMessageReceivedAgoMs: null,
    lastMessageSentAgoMs: null,
    ...overrides,
  };
}

function memoryStorage(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (name: string) => map.get(name) ?? null,
    setItem: (name: string, value: string) => {
      map.set(name, value);
    },
  };
}

describe('邀请身份', () => {
  it('只接受规范小写 UUIDv7', () => {
    expect(isCanonicalUuidV7(key)).toBe(true);
    expect(isCanonicalUuidV7(key.toUpperCase())).toBe(false);
    expect(isCanonicalUuidV7('0198b601-77a1-4bb8-83eb-a8fe68c97e44')).toBe(false);
  });

  it('显示名去掉首尾空白，拒绝空值、控制字符和超长', () => {
    expect(normalizeInviteDisplayName('  Scout  ')).toBe('Scout');
    expect(normalizeInviteDisplayName('   ')).toBeNull();
    expect(normalizeInviteDisplayName('a\u0007b')).toBeNull();
    expect(normalizeInviteDisplayName('名'.repeat(129))).toBeNull();
    expect(normalizeInviteDisplayName('名'.repeat(128))).toHaveLength(128);
  });

  it('按宿主持久化，损坏或不合法的记录视为不存在', () => {
    const storage = memoryStorage({
      [inviteIdentityStorageKey('cursor')]: '{"sessionKey":"bad","displayName":"x"}',
      [inviteIdentityStorageKey('other')]: 'not json',
    });
    const storageKey = inviteIdentityStorageKey('codex');
    expect(storageKey).toBe('agent-room.agent-invite.codex');
    expect(readInviteIdentity(storage, storageKey, 'p1')).toBeNull();
    writeInviteIdentity(storage, storageKey, {
      sessionKey: key,
      displayName: 'My Codex',
      ownerId: 'p1',
    });
    expect(readInviteIdentity(storage, storageKey, 'p1')).toEqual({
      sessionKey: key,
      displayName: 'My Codex',
      ownerId: 'p1',
    });
    expect(readInviteIdentity(storage, inviteIdentityStorageKey('cursor'), 'p1')).toBeNull();
    expect(readInviteIdentity(storage, inviteIdentityStorageKey('other'), 'p1')).toBeNull();
  });

  it('换账号不复用别人的身份；账号未知时沿用已保存的身份', () => {
    const storage = memoryStorage();
    const storageKey = inviteIdentityStorageKey('codex');
    writeInviteIdentity(storage, storageKey, {
      sessionKey: key,
      displayName: 'My Codex',
      ownerId: 'p1',
    });
    expect(readInviteIdentity(storage, storageKey, 'p2')).toBeNull();
    expect(readInviteIdentity(storage, storageKey, null)).toMatchObject({ sessionKey: key });
    writeInviteIdentity(storage, storageKey, {
      sessionKey: otherKey,
      displayName: 'Legacy',
      ownerId: null,
    });
    expect(readInviteIdentity(storage, storageKey, 'p2')).toEqual({
      sessionKey: otherKey,
      displayName: 'Legacy',
      ownerId: 'p2',
    });
  });

  it('存储抛错时读写都不会让界面崩溃', () => {
    const broken = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
    };
    expect(readInviteIdentity(broken, 'k', null)).toBeNull();
    expect(() => {
      writeInviteIdentity(broken, 'k', { sessionKey: key, displayName: 'x', ownerId: null });
    }).not.toThrow();
  });
});

describe('defaultInviteHost', () => {
  it('优先选择已安装的宿主，否则回退到通用配置', () => {
    const detection = (host: 'codex' | 'claude-code' | 'cursor', installed: boolean) => ({
      host,
      installed,
      configurable: installed,
      mechanism: 'config-file',
      diagnosticCode: 'host.detected',
    });
    expect(defaultInviteHost([])).toBe('other');
    expect(defaultInviteHost([detection('codex', false), detection('cursor', true)])).toBe(
      'cursor',
    );
    expect(defaultInviteHost([detection('claude-code', true), detection('codex', true)])).toBe(
      'codex',
    );
  });
});

describe('projectInviteStatus', () => {
  it('没有匹配 sessionKey 的会话时保持等待，不把别的任务算作成功', () => {
    expect(projectInviteStatus([], key)).toEqual({ kind: 'waiting' });
    expect(projectInviteStatus([session({ sessionKey: otherKey })], key)).toEqual({
      kind: 'waiting',
    });
    expect(projectInviteStatus([session({ sessionKey: null })], key)).toEqual({ kind: 'waiting' });
  });

  it('按会话状态映射，并只在近期取信时标记活跃', () => {
    expect(
      projectInviteStatus([session({ session: { ...session().session, state: 'starting' } })], key),
    ).toEqual({ kind: 'starting', displayName: 'Scout' });
    expect(projectInviteStatus([session()], key)).toEqual({
      kind: 'ready',
      displayName: 'Scout',
      sessionId: '0198b601-77a1-7bb8-83eb-a8fe68c97e50',
      active: true,
    });
    expect(projectInviteStatus([session({ lastInboxReadAgoMs: 60_000 })], key)).toMatchObject({
      kind: 'ready',
      active: false,
    });
    expect(
      projectInviteStatus(
        [
          session({
            session: { ...session().session, state: 'failed', errorCode: 'bridge.session.denied' },
          }),
        ],
        key,
      ),
    ).toEqual({ kind: 'failed', displayName: 'Scout', code: 'bridge.session.denied' });
    expect(
      projectInviteStatus([session({ session: { ...session().session, state: 'closed' } })], key),
    ).toEqual({ kind: 'closed', displayName: 'Scout' });
  });
});
