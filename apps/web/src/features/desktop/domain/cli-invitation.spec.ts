import { describe, expect, it } from 'vitest';
import { cliInvocation, encodeCliInvitation, quoteCliArgument } from './cli-invitation';

const identity = {
  sessionKey: '0198b601-77a1-7bb8-83eb-a8fe68c97e44',
  displayName: '中文 Agent 🌱',
  ownerId: 'owner',
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
  it('房间名这类参数用单引号：PowerShell 和 POSIX 各按自己的写法转义', () => {
    expect(quoteCliArgument("Rainy's room", 'windows')).toBe("'Rainy''s room'");
    expect(quoteCliArgument("Rainy's room", 'linux')).toBe("'Rainy'\"'\"'s room'");
    expect(quoteCliArgument('game dev', 'unknown')).toBe("'game dev'");
  });
});
