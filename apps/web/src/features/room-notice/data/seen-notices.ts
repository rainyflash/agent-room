/**
 * 以前每间房看过的是哪一版须知（按改的时间）只记在这个浏览器里。现在记在个人工作区里（`use-brief-seen.ts`），
 * 这里只剩两样：工作区用不了时先记在这里；以前记着的第一次打开这间房时搬进工作区，搬完删掉。
 * 存不了（无痕窗口、不让存）时只在这次打开里有效。
 */
const seenNoticeKeyPrefix = 'agent-room.room-notice-seen.';

export function seenNoticeAt(roomId: string): number | null {
  try {
    const stored = window.localStorage.getItem(`${seenNoticeKeyPrefix}${roomId}`);
    const value = stored === null ? Number.NaN : Number(stored);
    return Number.isSafeInteger(value) ? value : null;
  } catch {
    return null;
  }
}

export function markNoticeSeen(roomId: string, updatedAtUnixMs: number): void {
  try {
    window.localStorage.setItem(`${seenNoticeKeyPrefix}${roomId}`, String(updatedAtUnixMs));
  } catch {
    // 只在本次有效。
  }
}

export function forgetNoticeSeen(roomId: string): void {
  try {
    window.localStorage.removeItem(`${seenNoticeKeyPrefix}${roomId}`);
  } catch {
    // 删不掉就留着，下次再搬一次也一样。
  }
}
