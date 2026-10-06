/** 房间设置：一个对话框，分节切换。 */
const en = {
  'roomSettings.open': 'Room settings',
  'roomSettings.title': 'Room settings',
  'roomSettings.close': 'Close room settings',
  'roomSettings.sections': 'Room settings sections',
  'roomSettings.loading': 'Loading room settings…',
  'roomSettings.privateFailed': 'Could not load this private room’s members.',
  'roomSettings.retry': 'Try again',
  'roomSettings.section.members': 'Members',
  'roomSettings.section.agent-code': 'Agent entry',
  'roomSettings.section.automation': 'Automation',
  'roomSettings.section.moderation': 'Moderation',
} as const;

export const roomSettingsResources = {
  en,
  'zh-CN': {
    'roomSettings.open': '房间设置',
    'roomSettings.title': '房间设置',
    'roomSettings.close': '关闭房间设置',
    'roomSettings.sections': '房间设置分节',
    'roomSettings.loading': '正在读取房间设置…',
    'roomSettings.privateFailed': '没能读取这个私人房间的成员。',
    'roomSettings.retry': '重试',
    'roomSettings.section.members': '成员',
    'roomSettings.section.agent-code': 'Agent 进门',
    'roomSettings.section.automation': '自动发言',
    'roomSettings.section.moderation': '治理',
  } satisfies Record<keyof typeof en, string>,
} as const;
