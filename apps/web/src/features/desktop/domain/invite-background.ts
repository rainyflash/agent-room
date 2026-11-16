import type { HostSessionDiagnostics } from './desktop-runtime';

/**
 * 接入对话框里“你不在时也让它回话”（`specs/redesign/reminders.md` 的 4R-7）：刚进来的 Agent 登记了
 * 这间房的后台回复以后，对话框替人打开，和“我的 Agent”里那个按钮是同一套。
 */
export type InviteBackgroundState =
  /** 进来了，还没登记：它的应用不支持时会一直这样。 */
  | { readonly kind: 'waiting' }
  | { readonly kind: 'enabling' }
  | { readonly kind: 'on' }
  | { readonly kind: 'failed'; readonly code: string };

type BackgroundRoom = { readonly roomId: string; readonly catalogId?: string } | null;

/** 这个会话登记的后台回复就是这间房的（没有房间时是它进的公共大厅），对话框可以替人打开。 */
export function offersInviteBackground(
  session: HostSessionDiagnostics,
  room: BackgroundRoom,
): boolean {
  const offer = session.receptionOffer;
  if (
    offer?.roomCatalogId == null ||
    session.session.agentId === null ||
    session.session.state !== 'ready'
  ) {
    return false;
  }
  if (room === null) return true;
  return offer.roomId === room.roomId || offer.roomCatalogId === room.catalogId;
}

const inviteBackgroundKey = 'agent-room.invite-background';

/** 默认勾上；人关掉过就记在这台设备上，下次打开还是关着。读不到（隐私模式等）时照默认。 */
export function readInviteBackground(storage: Storage | null): boolean {
  try {
    return storage?.getItem(inviteBackgroundKey) !== 'off';
  } catch {
    return true;
  }
}

export function saveInviteBackground(storage: Storage | null, on: boolean): void {
  try {
    storage?.setItem(inviteBackgroundKey, on ? 'on' : 'off');
  } catch {
    // 只在这次有效。
  }
}
