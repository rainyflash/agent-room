export const accountResources = {
  en: {
    'settings.section.account': 'Account',
    'settings.account.signedInAs': 'Signed in as',
    'settings.account.export': 'Download my data',
    'settings.account.exportHint':
      'Your profile, computers, agents, rooms and reports, plus a list of your files, as a JSON file. Messages aren’t included.',
    'settings.account.exportAction': 'Download',
    'settings.account.exportPending': 'Preparing…',
    'settings.account.exportFailed': 'Couldn’t prepare the file. Try again in a moment. ({{code}})',
    'settings.account.delete': 'Delete account',
    'settings.account.deleteHint':
      'Deletes your account, your files and the text of your messages, and signs you out everywhere. This can’t be undone.',
    'settings.account.deleteOpen': 'Delete account…',
    'settings.account.deleteTitle': 'Delete your account?',
    'settings.account.deleteRemoved':
      'Your files and message texts are deleted, your chat account is closed, your computers lose access, and private rooms and agents that only you own are closed.',
    'settings.account.deleteKept':
      'People and agents in your rooms keep what they already received, other Matrix servers keep their copies, and backups keep everything for up to 30 days.',
    'settings.account.deleteSignIn': 'For your safety, sign in again before deleting your account.',
    'settings.account.deleteSignInAgain': 'Sign in again',
    'settings.account.deleteAcknowledge':
      'I understand that copies others already received can’t be deleted.',
    'settings.account.deleteType': 'Type {{word}} to confirm',
    'settings.account.deleteConfirm': 'Delete my account',
    'settings.account.deletePending': 'Deleting…',
    'settings.account.deleteCancel': 'Cancel',
    'settings.account.deleteFailed':
      'Your account wasn’t deleted. Try again in a moment. ({{code}})',
    'account.deleted.title': 'Your account is being deleted',
    'account.deleted.body':
      'You’ve been signed out. The rest is removed from the server within a few minutes.',
  },
  'zh-CN': {
    'settings.section.account': '账户',
    'settings.account.signedInAs': '当前登录',
    'settings.account.export': '下载我的数据',
    'settings.account.exportHint':
      '你的资料、电脑、Agent、房间、举报和文件清单，存成一个 JSON 文件。消息不在里面。',
    'settings.account.exportAction': '下载',
    'settings.account.exportPending': '正在准备…',
    'settings.account.exportFailed': '文件没准备好，稍后再试。（{{code}}）',
    'settings.account.delete': '删除账户',
    'settings.account.deleteHint':
      '删除你的账户、文件和消息正文，所有设备都会退出登录。删了就找不回来。',
    'settings.account.deleteOpen': '删除账户…',
    'settings.account.deleteTitle': '确定删除账户？',
    'settings.account.deleteRemoved':
      '你的文件和消息正文会删除，聊天账户会关闭，电脑的授权失效，只有你拥有的私人房间和 Agent 会关闭。',
    'settings.account.deleteKept':
      '房间里的人和 Agent 已经收到的会留在他们那里，其他 Matrix 服务器上的副本也删不掉，备份最多再留 30 天。',
    'settings.account.deleteSignIn': '为了安全，删除账户前请重新登录一次。',
    'settings.account.deleteSignInAgain': '重新登录',
    'settings.account.deleteAcknowledge': '我明白别人已经收到的副本删不掉。',
    'settings.account.deleteType': '输入 {{word}} 确认',
    'settings.account.deleteConfirm': '删除我的账户',
    'settings.account.deletePending': '正在删除…',
    'settings.account.deleteCancel': '取消',
    'settings.account.deleteFailed': '账户没有删掉，稍后再试。（{{code}}）',
    'account.deleted.title': '正在删除你的账户',
    'account.deleted.body': '你已经退出登录，服务器上剩下的几分钟内删完。',
  },
} as const;
