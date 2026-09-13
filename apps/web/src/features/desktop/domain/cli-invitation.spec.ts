import { describe, expect, it } from 'vitest';
import {
  cliInvocation,
  encodeCliInvitation,
  readInviteHistory,
  saveInviteHistory,
} from './cli-invitation';

const identity = {
  sessionKey: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
  displayName: '中文 Agent 🌱',
  ownerId: 'owner',
};
const storage = () => {
  const entries = new Map<string, string>();
  return {
    getItem: (key: string) => entries.get(key) ?? null,
    setItem: (key: string, value: string) => {
      entries.set(key, value);
    },
  };
};

describe('CLI invitations', () => {
  it('编码只含接入元数据，中文可还原，不携带账号或密钥', () => {
    const encoded = encodeCliInvitation(identity, '!room:test');
    expect(encoded).toMatch(/^[A-Za-z0-9_-]+$/u);
    const decoded: unknown = JSON.parse(
      new TextDecoder().decode(
        Uint8Array.from(atob(encoded.replaceAll('-', '+').replaceAll('_', '/')), (value) =>
          value.charCodeAt(0),
        ),
      ),
    );
    expect(decoded).toEqual({
      version: 1,
      sessionKey: identity.sessionKey,
      displayName: identity.displayName,
      roomId: '!room:test',
    });
  });
  it('以实际安装位置生成 PowerShell 和 POSIX 命令并引用特殊字符', () => {
    const configuration = {
      command: "C:\\A ' B $HOME\\agent-room.exe",
      args: ['--data-root', "C:\\X ' Y"],
    };
    expect(cliInvocation(configuration, 'windows')).toBe(
      "& 'C:\\A '' B $HOME\\agent-room.exe' '--data-root' 'C:\\X '' Y'",
    );
    expect(cliInvocation({ command: "/a'b/agent-room", args: [] }, 'linux')).toBe(
      "'/a'\"'\"'b/agent-room'",
    );
    expect(cliInvocation(null, 'unknown')).toBe('agent-room');
  });
  it('按账号保存多个人物并迁移明确属于该账号的旧人物，不自动接管', () => {
    const values = storage();
    values.setItem('agent-room.agent-invite.codex', JSON.stringify(identity));
    expect(readInviteHistory(values, 'owner').identities).toEqual([identity]);
    expect(readInviteHistory(values, 'other').identities).toEqual([]);
    expect(readInviteHistory(values, null).identities).toEqual([]);
    const second = { ...identity, sessionKey: '0198b601-77a1-7bb8-83eb-a8fe68c97e45' };
    expect(saveInviteHistory(values, second)).toBe(true);
    expect(readInviteHistory(values, 'owner').identities).toEqual([second, identity]);
    expect(saveInviteHistory(values, identity)).toBe(true);
    expect(readInviteHistory(values, 'owner').identities).toEqual([identity, second]);
  });
  it('损坏和存储失败显式返回，不能覆盖旧记录或伪装保存成功', () => {
    const values = storage();
    values.setItem('agent-room.invitations.v2.owner', '{');
    expect(readInviteHistory(values, 'owner').unavailable).toBe(true);
    expect(saveInviteHistory(values, identity)).toBe(false);
    expect(values.getItem('agent-room.invitations.v2.owner')).toBe('{');
    const broken = {
      ...storage(),
      setItem: () => {
        throw new Error('quota');
      },
    };
    expect(saveInviteHistory(broken, identity)).toBe(false);
  });
  it('保存的邀请保留目标房间，避免从其他页面恢复时偷偷改绑', () => {
    const values = storage();
    const invited = {
      ...identity,
      room: { roomId: '!room:test', roomName: 'Studio', catalogId: identity.sessionKey },
    };
    expect(saveInviteHistory(values, invited)).toBe(true);
    expect(readInviteHistory(values, 'owner').identities[0]).toEqual(invited);
    const encoded = encodeCliInvitation(invited, invited.room.roomId, invited.room.catalogId);
    expect(JSON.parse(atob(encoded.replaceAll('-', '+').replaceAll('_', '/')))).toMatchObject({
      roomId: '!room:test',
      catalogId: identity.sessionKey,
    });
  });
});
