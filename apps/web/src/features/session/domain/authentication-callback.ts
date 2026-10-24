export type AuthenticationCallbackFailure = 'expired' | 'failed';

export function authenticationCallbackFailure(path: string): AuthenticationCallbackFailure | null {
  if (!/^\/connect(?:[?#]|$)/u.test(path)) return null;
  const query = new URLSearchParams(path.split('?')[1]?.split('#')[0]);
  const reason = query.get('authentication');
  return reason === 'expired' || reason === 'failed' ? reason : null;
}

/** 自动重来过以后多久之内再失败，就停在提示上让人自己点，免得在登录页和这里来回跳。 */
export const AUTOMATIC_RESTART_WINDOW_MS = 5 * 60_000;
const AUTOMATIC_RESTART_KEY = 'agent-room.authentication.automatic-restart';

/**
 * 没登录的人登录没走完（身份服务把错误送回回调、浏览器里留着的旧登录地址、登录等太久）时，
 * 替他重新打开一次登录页。记在标签页的 sessionStorage 里：登录页和回调都在这个标签页里走，
 * 读写不了就不自动重来。
 */
export function claimAutomaticRestart(
  storage: Pick<Storage, 'getItem' | 'setItem'>,
  nowMs: number,
): boolean {
  try {
    const previous = Number(storage.getItem(AUTOMATIC_RESTART_KEY));
    const elapsed = nowMs - previous;
    if (previous > 0 && elapsed >= 0 && elapsed < AUTOMATIC_RESTART_WINDOW_MS) return false;
    storage.setItem(AUTOMATIC_RESTART_KEY, String(nowMs));
    return true;
  } catch {
    return false;
  }
}
