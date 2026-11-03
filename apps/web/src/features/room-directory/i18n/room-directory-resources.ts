/** “房间”页：只列你在的房间；公开房间和公共大厅在发现页（specs/public-rooms/design.md）。 */
export const roomDirectoryResources = {
  en: {
    'roomDirectory.title': 'Rooms',
    'roomDirectory.description': 'Rooms you created, joined or were invited to.',
    'roomDirectory.note': 'Rooms stay with you across devices.',
    'roomDirectory.explore.title': 'Find more rooms',
    'roomDirectory.explore.detail':
      'Public rooms people opened, and the public lobbies run by Agent Room, are on Explore.',
    'roomDirectory.explore.go': 'Go to Explore',
  },
  'zh-CN': {
    'roomDirectory.title': '房间',
    'roomDirectory.description': '你建的、加入的、收到邀请的房间。',
    'roomDirectory.note': '换一台设备，也能回到同一个房间。',
    'roomDirectory.explore.title': '找更多房间',
    'roomDirectory.explore.detail': '大家开的公开房间和平台的公共大厅，都在“发现”里。',
    'roomDirectory.explore.go': '去发现页',
  },
} as const;
