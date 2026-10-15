import { parseInviteSecret } from '@/features/private-rooms/domain/private-room-invite-link';

/** 打开邀请链接的页面。链接是 `<网页>/join#<秘密>`。 */
export const JOIN_INVITE_PATH = '/join';

const storageKey = 'agent-room.private-room-invite';

type InviteLocation = Pick<Location, 'hash' | 'pathname' | 'search'>;
type InviteHistory = Pick<History, 'replaceState' | 'state'>;
type InviteStorage = Pick<Storage, 'getItem' | 'removeItem' | 'setItem'>;

// 存不进 sessionStorage 时（隐私模式、禁用了网站数据）这一页里照样用得上，只是登录跳转回来就没了。
let remembered: string | null = null;

function sessionStorageOrNull(): InviteStorage | null {
  try {
    return window.sessionStorage;
  } catch {
    return null;
  }
}

/**
 * 页面渲染之前调用：打开 `/join#秘密` 时，把秘密收进这个标签页，再从地址栏去掉。
 * 登录要跳走时会把当前地址当成回来的地方带上，秘密不能跟着走；`#` 后面那段本来就不会发给服务器。
 * 秘密不像样的也去掉，并忘掉之前收着的，免得打开的是坏链接、用的却是上一条。
 */
export function stashInviteSecret(
  location: InviteLocation = window.location,
  history: InviteHistory = window.history,
  storage: InviteStorage | null = sessionStorageOrNull(),
): void {
  if (location.pathname !== JOIN_INVITE_PATH || location.hash.length <= 1) return;
  const secret = parseInviteSecret(location.hash.slice(1));
  remembered = secret;
  try {
    if (secret === null) storage?.removeItem(storageKey);
    else storage?.setItem(storageKey, secret);
  } catch {
    // 只留在这一页里。
  }
  history.replaceState(history.state, '', `${location.pathname}${location.search}`);
}

/** 这个标签页收着的秘密；登录跳转回来以后也还在。没有时为空。 */
export function readInviteSecret(
  storage: InviteStorage | null = sessionStorageOrNull(),
): string | null {
  try {
    const stored = storage?.getItem(storageKey) ?? null;
    if (stored !== null) return parseInviteSecret(stored);
  } catch {
    // 用这一页里记着的。
  }
  return remembered;
}

/** 用过了、或者链接已经没用了：忘掉。 */
export function forgetInviteSecret(storage: InviteStorage | null = sessionStorageOrNull()): void {
  remembered = null;
  try {
    storage?.removeItem(storageKey);
  } catch {
    // 不算错。
  }
}
