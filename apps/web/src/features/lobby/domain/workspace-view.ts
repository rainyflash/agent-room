export const roomWorkspaceViews = ['conversation', 'resources', 'space'] as const;
export type RoomWorkspaceView = (typeof roomWorkspaceViews)[number];

/**
 * 房间页的两种版面（`specs/redesign/room-page.md`）：对话占主栏、Agent 在右边一栏；或者原来的场景。
 * 场景里是画面还是名册归账户偏好的 `lobbyView` 管，和这个不是一回事。
 */
export const roomPageLayouts = ['conversation', 'scene'] as const;
export type RoomPageLayout = (typeof roomPageLayouts)[number];

export function isRoomPageLayout(value: unknown): value is RoomPageLayout {
  return value === 'conversation' || value === 'scene';
}
