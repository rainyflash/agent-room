import { isRoomPageLayout, type RoomPageLayout } from '@/features/lobby/domain/workspace-view';

/**
 * 房间页用哪种版面记在这台设备上，不进账户偏好：账户偏好的 `lobbyView` 只认 `scene`、`list`，
 * 旧版客户端读到第三个值会出错（`specs/redesign/room-page.md`）。
 */
const storageKey = 'agent-room.room-page-layout';

/** 地址里没写、这台设备也没选过时用哪个。 */
export const defaultRoomPageLayout: RoomPageLayout = 'conversation';

type LayoutStorage = Pick<Storage, 'getItem' | 'setItem'>;

function deviceStorage(): LayoutStorage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** 这台设备上次选的版面；没选过、读不了（隐私模式等）都是 null。 */
export function rememberedRoomPageLayout(
  storage: LayoutStorage | null = deviceStorage(),
): RoomPageLayout | null {
  try {
    const value = storage?.getItem(storageKey);
    return isRoomPageLayout(value) ? value : null;
  } catch {
    return null;
  }
}

/** 记不下来就只在这次打开里有效。 */
export function rememberRoomPageLayout(
  layout: RoomPageLayout,
  storage: LayoutStorage | null = deviceStorage(),
): void {
  try {
    storage?.setItem(storageKey, layout);
  } catch {
    // 只在本次有效。
  }
}

/** 地址里写了就照地址，不然看这台设备记着的，再不然用默认的。 */
export function resolveRoomPageLayout(
  requested: RoomPageLayout | undefined,
  remembered: RoomPageLayout | null,
): RoomPageLayout {
  return requested ?? remembered ?? defaultRoomPageLayout;
}
