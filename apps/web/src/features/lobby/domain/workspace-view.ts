export const roomWorkspaceViews = ['conversation', 'resources', 'space'] as const;
export type RoomWorkspaceView = (typeof roomWorkspaceViews)[number];

/**
 * 地址里的 `view` 能写的：对话、资料，再加对话版面里展开成整页的 Agent 表格（`agents`）。
 * 场景里没有那一页，写了也按画面算。
 */
export const roomPanelViews = ['conversation', 'resources', 'agents'] as const;
export type RoomPanelView = (typeof roomPanelViews)[number];

/** 房间页此刻在看什么：地址里写的那几种，不写就是画面本身（`space`）。 */
export type RoomPageView = RoomPanelView | 'space';

export function isRoomPanelView(value: unknown): value is RoomPanelView {
  return roomPanelViews.some((view) => view === value);
}

/**
 * 房间页的两种版面（`specs/redesign/room-page.md`）：对话占主栏、Agent 在右边一栏；或者原来的场景。
 * 场景里是画面还是名册归账户偏好的 `lobbyView` 管，和这个不是一回事。
 */
export const roomPageLayouts = ['conversation', 'scene'] as const;
export type RoomPageLayout = (typeof roomPageLayouts)[number];

export function isRoomPageLayout(value: unknown): value is RoomPageLayout {
  return value === 'conversation' || value === 'scene';
}
