import type { DesktopRuntimeSnapshot } from './desktop-runtime';

/**
 * 一个本机人物的身份：`sessionKey` 找回同一个人物，`displayName` 为空时由 Agent 自己起名，
 * `ownerId` 记下创建它的账号。接入对话框不再管人物；后台回复把任务交回别的电脑时还要用它编邀请。
 */
export type CliInvitationIdentity = {
  readonly sessionKey: string;
  readonly displayName: string;
  readonly ownerId: string | null;
};

export function encodeCliInvitation(
  identity: CliInvitationIdentity,
  roomId: string | null,
  catalogId?: string,
): string {
  const bytes = new TextEncoder().encode(
    JSON.stringify({
      version: 1,
      sessionKey: identity.sessionKey,
      ...(identity.displayName === '' ? {} : { displayName: identity.displayName }),
      roomId,
      ...(catalogId === undefined ? {} : { catalogId }),
    }),
  );
  return btoa(String.fromCharCode(...bytes))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replace(/=+$/u, '');
}

export function cliInvocation(
  configuration: DesktopRuntimeSnapshot['cliConfiguration'],
  platform: DesktopRuntimeSnapshot['platform'],
): string {
  if (configuration === null || configuration === undefined) return 'agent-room';
  const quote = (value: string) => quoteCliArgument(value, platform);
  return `${platform === 'windows' ? '& ' : ''}${[configuration.command, ...configuration.args].map(quote).join(' ')}`;
}

/**
 * 给命令行参数加单引号。PowerShell 和 POSIX shell 都把单引号里的内容当原文，只是转义单引号的写法
 * 不同；不知道 Agent 在哪种系统上时（网页端）用 POSIX 的写法。
 */
export function quoteCliArgument(
  value: string,
  platform: DesktopRuntimeSnapshot['platform'],
): string {
  return platform === 'windows'
    ? `'${value.replaceAll("'", "''")}'`
    : `'${value.replaceAll("'", "'\"'\"'")}'`;
}
