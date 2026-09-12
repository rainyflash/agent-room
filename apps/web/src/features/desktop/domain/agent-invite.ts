import type { AgentHostDetection, HostSessionDiagnostics } from './desktop-runtime';

/** 邀请面板可选择的宿主；`other` 表示任意支持 MCP stdio 的工具。 */
export type AgentInviteHost = 'codex' | 'claude-code' | 'cursor' | 'other';

export const agentInviteHosts: readonly AgentInviteHost[] = [
  'codex',
  'claude-code',
  'cursor',
  'other',
];

/**
 * 一个 Agent 在 Agent Room 中的专属身份；同一身份重复接入会恢复同一人物。
 * `ownerId` 记录创建时登录的账号；换账号后不复用别人的身份。
 */
export type AgentInviteIdentity = {
  readonly sessionKey: string;
  readonly displayName: string;
  readonly ownerId: string | null;
};

export type AgentInviteStatus =
  | { readonly kind: 'waiting' }
  | { readonly kind: 'starting'; readonly displayName: string }
  | {
      readonly kind: 'ready';
      readonly displayName: string;
      readonly sessionId: string;
      readonly active: boolean;
    }
  | { readonly kind: 'failed'; readonly displayName: string; readonly code: string | null }
  | { readonly kind: 'closed'; readonly displayName: string };

const canonicalUuidV7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;
const controlCharacters = /[\p{Cc}]/u;
const DISPLAY_NAME_MAX = 128;
/** 与桌面诊断一致：最近 35 秒内有取信才算活跃。 */
const ACTIVE_WINDOW_MS = 35_000;
const STORAGE_PREFIX = 'agent-room.agent-invite';

export function isCanonicalUuidV7(value: string): boolean {
  return canonicalUuidV7.test(value);
}

/** 返回可提交给 `agent_room_open_session` 的显示名；不合法时返回 null。 */
export function normalizeInviteDisplayName(value: string): string | null {
  const trimmed = value.trim();
  if (trimmed.length === 0 || codePointCount(trimmed) > DISPLAY_NAME_MAX) return null;
  if (controlCharacters.test(trimmed)) return null;
  return trimmed;
}

/** 与 MCP 的 `length(max = 128)` 一致：按 Unicode 码点计数，而不是 UTF-16 单元。 */
function codePointCount(value: string): number {
  return Array.from(value, () => 1).length;
}

/** 身份按设备和宿主保存：同一台电脑上无论从哪个页面接入，同一个工具都是同一个人物。 */
export function inviteIdentityStorageKey(host: AgentInviteHost): string {
  return `${STORAGE_PREFIX}.${host}`;
}

/**
 * 读取保存的身份。已知当前账号且与保存的账号不同，视为没有身份；
 * 账号未知（例如尚未进入会话页面）时沿用保存的身份。
 */
export function readInviteIdentity(
  storage: Pick<Storage, 'getItem'>,
  key: string,
  ownerId: string | null,
): AgentInviteIdentity | null {
  try {
    const raw = storage.getItem(key);
    if (raw === null) return null;
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== 'object' || parsed === null) return null;
    const candidate = parsed as {
      sessionKey?: unknown;
      displayName?: unknown;
      ownerId?: unknown;
    };
    if (typeof candidate.sessionKey !== 'string' || typeof candidate.displayName !== 'string') {
      return null;
    }
    if (!isCanonicalUuidV7(candidate.sessionKey)) return null;
    const displayName = normalizeInviteDisplayName(candidate.displayName);
    if (displayName === null) return null;
    const storedOwner = typeof candidate.ownerId === 'string' ? candidate.ownerId : null;
    if (ownerId !== null && storedOwner !== null && storedOwner !== ownerId) return null;
    return { sessionKey: candidate.sessionKey, displayName, ownerId: storedOwner ?? ownerId };
  } catch {
    return null;
  }
}

export function writeInviteIdentity(
  storage: Pick<Storage, 'setItem'>,
  key: string,
  identity: AgentInviteIdentity,
): void {
  try {
    storage.setItem(key, JSON.stringify(identity));
  } catch {
    // 存储不可用时身份只在本次会话内有效；界面已提示重新连接需复用同一指令。
  }
}

/** 默认选中已安装的宿主；都没有时选择通用配置。 */
export function defaultInviteHost(hosts: readonly AgentHostDetection[]): AgentInviteHost {
  for (const host of agentInviteHosts) {
    if (host !== 'other' && hosts.some((entry) => entry.host === host && entry.installed)) {
      return host;
    }
  }
  return 'other';
}

/** 只认当前邀请的 sessionKey；其他任务的会话不算这次邀请成功。 */
export function projectInviteStatus(
  sessions: readonly HostSessionDiagnostics[],
  sessionKey: string,
): AgentInviteStatus {
  const match = sessions.find((entry) => entry.sessionKey === sessionKey);
  if (match === undefined) return { kind: 'waiting' };
  switch (match.session.state) {
    case 'starting':
      return { kind: 'starting', displayName: match.displayName };
    case 'ready':
      return {
        kind: 'ready',
        displayName: match.displayName,
        sessionId: match.session.sessionId,
        active: match.lastInboxReadAgoMs !== null && match.lastInboxReadAgoMs < ACTIVE_WINDOW_MS,
      };
    case 'failed':
      return { kind: 'failed', displayName: match.displayName, code: match.session.errorCode };
    case 'closed':
      return { kind: 'closed', displayName: match.displayName };
  }
}
