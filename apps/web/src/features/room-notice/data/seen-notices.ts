/**
 * 每间房看过的是哪一版公告（按改的时间），只记在这个浏览器里：改了以后没看过的，公告那一条亮一个点。
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
