export const settingsResources = {
  en: {
    'settings.title': 'Settings',
    'settings.sections': 'Settings sections',
    'settings.section.general': 'General',
    'settings.section.security': 'Security',
    'settings.section.thisComputer': 'This computer',
    'settings.section.about': 'About',
    'settings.general.language': 'Language',
    'settings.general.languageHint':
      'Your account’s language follows you to every device. Pick “Only on this device” to change just this one.',
    'settings.general.roomView': 'Rooms open in',
    'settings.general.roomViewHint': 'You can still switch views inside a room.',
    'settings.general.roomView.scene': 'Scene',
    'settings.general.roomView.list': 'List',
    'settings.general.reminders': 'Reminders',
    'settings.general.webNotifications': 'Web notifications',
    'settings.general.webNotificationsHint':
      'When this tab isn’t in front, a system notification pops up if someone mentions you, replies to you or messages you. Only in this browser.',
    'settings.general.webNotificationsDenied':
      'This browser won’t let the site send notifications. To turn them on, open the site information at the left of the address bar, allow notifications, then come back here.',
    'settings.general.webNotificationsUnsupported':
      'This browser can’t show notifications from web pages. On an iPhone, add Agent Room to the Home Screen first.',
    'settings.general.emailReminders': 'Email reminders',
    'settings.general.emailRemindersHint':
      'If someone says something and you haven’t looked within an hour, you get an email listing the rooms with new messages. At most one a day, never the messages themselves.',
    'settings.general.quiet': 'Pause notifications',
    'settings.general.quietHint':
      'New messages still arrive in your inbox; they just don’t pop up.',
    'settings.general.quietHintEmail':
      'New messages still arrive in your inbox; they just don’t pop up, and no reminder emails go out.',
    'settings.general.quietOn': 'Paused until {{time}}',
    'settings.general.quietOnEmail':
      'Paused until {{time}}. No pop-ups and no reminder emails until then; afterwards, emails only mention what came in after the pause.',
    'settings.general.quietHour': 'Pause for an hour',
    'settings.general.quietResume': 'Resume now',
    'settings.general.quietFailed': 'Couldn’t save this setting. Try again.',
    'settings.thisComputer.intro': 'These settings apply only to Agent Room on this computer.',
    'settings.thisComputer.agents': 'See the agents connected here',
    'settings.updateDot': 'Update ready',
    'toasts.label': 'Notifications',
    'toasts.update.title': 'Agent Room {{version}} is ready to install',
    'toasts.update.install': 'Update and restart',
    'toasts.update.later': 'Later',
    'toasts.update.failed': 'The update didn’t install. Check your connection and try again.',
  },
  'zh-CN': {
    'settings.title': '设置',
    'settings.sections': '设置分节',
    'settings.section.general': '通用',
    'settings.section.security': '安全',
    'settings.section.thisComputer': '这台电脑',
    'settings.section.about': '关于',
    'settings.general.language': '语言',
    'settings.general.languageHint': '账户的语言会跟着你到每台设备；选“仅此设备”只改这一台。',
    'settings.general.roomView': '进房间时显示',
    'settings.general.roomViewHint': '在房间里也可以随时切换。',
    'settings.general.roomView.scene': '场景',
    'settings.general.roomView.list': '列表',
    'settings.general.reminders': '提醒',
    'settings.general.webNotifications': '网页通知',
    'settings.general.webNotificationsHint':
      '这个标签页不在前面时，有人提到你、回复你、私聊你，就弹一条系统通知。只管这个浏览器。',
    'settings.general.webNotificationsDenied':
      '这个浏览器不让本站发通知。要打开，点地址栏左边的网站信息，把“通知”改成允许，再回来打开这里。',
    'settings.general.webNotificationsUnsupported':
      '这个浏览器不支持网页通知。iPhone 上要先把 Agent Room 加到主屏幕。',
    'settings.general.emailReminders': '邮件提醒',
    'settings.general.emailRemindersHint':
      '有人说了话、你一小时还没看时，发一封邮件写哪几间房有新消息。每天最多一封，不写消息内容。',
    'settings.general.quiet': '暂停提醒',
    'settings.general.quietHint': '新消息照样进收件箱，只是不弹出来。',
    'settings.general.quietHintEmail': '新消息照样进收件箱，只是不弹出来，也不发提醒邮件。',
    'settings.general.quietOn': '暂停到 {{time}}',
    'settings.general.quietOnEmail':
      '暂停到 {{time}}。这段时间不弹通知，也不发提醒邮件；恢复以后，邮件只说恢复以后来的。',
    'settings.general.quietHour': '暂停一小时',
    'settings.general.quietResume': '现在恢复',
    'settings.general.quietFailed': '没能保存这项设置，请再试一次。',
    'settings.thisComputer.intro': '这些设置只对这台电脑上的 Agent Room 有效。',
    'settings.thisComputer.agents': '查看接在这里的 Agent',
    'settings.updateDot': '有新版本',
    'toasts.label': '通知',
    'toasts.update.title': '新版本 {{version}} 可以安装了',
    'toasts.update.install': '更新并重启',
    'toasts.update.later': '稍后',
    'toasts.update.failed': '更新没装上。检查网络后再试一次。',
  },
} as const;
