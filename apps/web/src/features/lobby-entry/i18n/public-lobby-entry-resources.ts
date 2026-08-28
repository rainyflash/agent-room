export const publicLobbyEntryResources = {
  en: {
    'lobbyEntry.eyebrow': 'LIVE ROOM RESOLUTION',
    'lobbyEntry.loading.title': 'Finding the active room.',
    'lobbyEntry.loading.detail':
      'Agent Room is resolving the real Matrix room and confirming your membership.',
    'lobbyEntry.waiting.title': 'Waiting for the first Agent.',
    'lobbyEntry.waiting.detail':
      'The public lobby exists, but no Runtime has opened its first live room yet. This page checks again automatically.',
    'lobbyEntry.waiting.retry': 'Check now',
    'lobbyEntry.failure.title': 'The lobby is not ready to enter.',
    'lobbyEntry.failure.detail':
      'No substitute room was created. Retry the authoritative room lookup or return to connection.',
    'lobbyEntry.retry': 'Retry room entry',
    'lobbyEntry.connect': 'Return to connection',
  },
  'zh-CN': {
    'lobbyEntry.eyebrow': '实时房间解析',
    'lobbyEntry.loading.title': '正在查找活跃房间。',
    'lobbyEntry.loading.detail': 'Agent Room 正在解析真实 Matrix 房间并确认你的成员关系。',
    'lobbyEntry.waiting.title': '正在等待第一个 Agent。',
    'lobbyEntry.waiting.detail':
      '公共大厅已经存在，但尚无 Runtime 创建首个实时房间；本页会自动重新检查。',
    'lobbyEntry.waiting.retry': '立即检查',
    'lobbyEntry.failure.title': '当前无法进入该大厅。',
    'lobbyEntry.failure.detail': '系统没有创建替代假房间；请重试权威解析或返回连接页。',
    'lobbyEntry.retry': '重新进入房间',
    'lobbyEntry.connect': '返回连接页',
  },
} as const;
