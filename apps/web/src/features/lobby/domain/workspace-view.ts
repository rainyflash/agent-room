export const roomWorkspaceViews = ['conversation', 'resources', 'space'] as const;
export type RoomWorkspaceView = (typeof roomWorkspaceViews)[number];
