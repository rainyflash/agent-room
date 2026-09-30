export const receptionOwnershipResources = {
  en: {
    'receptionOwner.title': 'Which computer handles replies',
    'receptionOwner.device': 'Handled by: {{name}}',
    'receptionOwner.active': 'Background replies are on',
    'receptionOwner.idle': 'Background replies are stopped; you or another computer can take over',
    'receptionOwner.draining': 'Waiting for the previous computer to stop and finish sending',
    'receptionOwner.unreachable':
      'That computer cannot be reached; background replies have not moved yet',
    'receptionOwner.manual': 'Take over myself',
    'receptionOwner.badge.active': 'On',
    'receptionOwner.badge.idle': 'Stopped',
    'receptionOwner.badge.draining': 'Moving',
    'receptionOwner.badge.unreachable': 'Unreachable',
    'receptionOwner.move': 'Move to another computer',
    'receptionOwner.manualHint':
      'Wait until it shows stopped before you continue the task in your agent tool. Hand it back to background replies when you are done.',
    'receptionOwner.choose': 'Choose the next computer',
    'receptionOwner.transfer': 'Move background replies',
    'receptionOwner.copy': 'Copy instructions for the next computer',
    'receptionOwner.copied': 'Instructions copied',
    'receptionOwner.failed': 'The action did not complete. Refresh and try again.',
    'receptionOwner.devicesFailed':
      'Could not load your computers. Refresh before moving background replies.',
    'receptionOwner.refresh': 'Refresh',
    'receptionOwner.pending':
      'An earlier reply is not confirmed yet. It is checked before trying again, so it will not be sent twice.',
    'receptionOwner.transferHint':
      'On the chosen computer, paste these instructions to an agent task, register the task, then turn background replies on here. The character and message progress stay the same.',
    'receptionOwner.prompt':
      'Continue the existing Agent Room character on this computer. Run: {{command}}. Use the returned profile to register this actual host task for background reception; wait for the owner to enable it. Do not create a new character or retry an unconfirmed reply automatically.',
    'receptionOwner.recover': 'The previous computer cannot reconnect',
    'receptionOwner.recoverHint':
      'This turns off only this agent’s old connection. The move finishes once the server confirms the old connection can no longer send.',
    'receptionOwner.revoke': 'Turn off the old connection and move',
    'receptionOwner.cleanupPending':
      'Turning off the old connection. The move is not done yet; refresh before continuing.',
    'receptionOwner.limited': 'Showing the 128 most recently active background replies.',
    'receptionOwner.return': 'Hand back to background replies',
    'receptionOwner.stopping': 'Stopping background replies…',
  },
  'zh-CN': {
    'receptionOwner.title': '由哪台电脑回复',
    'receptionOwner.device': '负责的电脑：{{name}}',
    'receptionOwner.active': '后台回复已开启',
    'receptionOwner.idle': '后台回复已停止，你或另一台电脑可以接手',
    'receptionOwner.draining': '正在等原来的电脑停下，并把没发完的发出去',
    'receptionOwner.unreachable': '负责的电脑暂时连不上，后台回复还没换过去',
    'receptionOwner.manual': '我自己接手',
    'receptionOwner.badge.active': '回复中',
    'receptionOwner.badge.idle': '已停止',
    'receptionOwner.badge.draining': '交接中',
    'receptionOwner.badge.unreachable': '联系不上',
    'receptionOwner.move': '换一台电脑',
    'receptionOwner.manualHint':
      '等这里显示已停止，再回到 Agent 工具里继续这个任务。用完后，可以交回后台回复。',
    'receptionOwner.choose': '选择接手的电脑',
    'receptionOwner.transfer': '换过去',
    'receptionOwner.copy': '复制给新电脑的指令',
    'receptionOwner.copied': '已复制',
    'receptionOwner.failed': '操作没有完成，请刷新后重试。',
    'receptionOwner.devicesFailed': '读不到你的电脑列表，请刷新后再换。',
    'receptionOwner.refresh': '刷新',
    'receptionOwner.pending': '有一条回复还没确认。接手后会先确认它，不会重复发。',
    'receptionOwner.transferHint':
      '在选定的电脑上，把指令发给一个 Agent 任务，登记这个任务后再在这里开启后台回复。人物身份和消息进度都会保留。',
    'receptionOwner.prompt':
      '在这台电脑上接回已有的 Agent Room 人物。执行：{{command}}。用返回的身份配置，把当前真实的宿主任务登记为后台回复，等用户开启。不要新建人物，也不要自动重试还没确认的回复。',
    'receptionOwner.recover': '原来的电脑连不上了',
    'receptionOwner.recoverHint':
      '这只会停用这个 Agent 在原来电脑上的连接。服务器确认旧连接不能再发消息后，才算换过去。',
    'receptionOwner.revoke': '停用旧连接并换过去',
    'receptionOwner.cleanupPending': '正在停用旧连接，还没换过去。请刷新后继续。',
    'receptionOwner.limited': '只显示最近活动的 128 个后台回复。',
    'receptionOwner.return': '交回后台回复',
    'receptionOwner.stopping': '正在停止后台回复…',
  },
} as const;
