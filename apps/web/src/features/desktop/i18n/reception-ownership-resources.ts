export const receptionOwnershipResources = {
  en: {
    'receptionOwner.title': 'Who is receiving messages',
    'receptionOwner.device': 'Responsible computer: {{name}}',
    'receptionOwner.active': 'Background reception is connected',
    'receptionOwner.idle':
      'Background reception has stopped; ready for a person or another computer',
    'receptionOwner.draining':
      'Waiting for the previous computer to stop and finish outbound messages',
    'receptionOwner.unreachable':
      'Responsible computer is unreachable; reception has not been transferred',
    'receptionOwner.manual': 'Take over manually',
    'receptionOwner.badge.active': 'Receiving',
    'receptionOwner.badge.idle': 'Stopped',
    'receptionOwner.badge.draining': 'Handing over',
    'receptionOwner.badge.unreachable': 'Unreachable',
    'receptionOwner.move': 'Move reception to another computer',
    'receptionOwner.manualHint':
      'Wait until reception has stopped before continuing this task in your Agent tool. Return it to background reception when you finish.',
    'receptionOwner.choose': 'Choose the next computer',
    'receptionOwner.transfer': 'Transfer reception',
    'receptionOwner.copy': 'Copy instructions for the next computer',
    'receptionOwner.copied': 'Instructions copied',
    'receptionOwner.failed': 'The action did not complete. Refresh and try again.',
    'receptionOwner.devicesFailed':
      'Could not load your computers. Refresh before transferring reception.',
    'receptionOwner.refresh': 'Refresh reception status',
    'receptionOwner.pending':
      'An earlier reply is unconfirmed. Its delivery identity will be retained and checked before retrying.',
    'receptionOwner.transferHint':
      'On the selected computer, paste these instructions into an Agent task, register that task for reception, then enable it here. The character and message progress stay the same.',
    'receptionOwner.prompt':
      'Continue the existing Agent Room character on this computer. Run: {{command}}. Use the returned profile to register this actual host task for background reception; wait for the owner to enable it. Do not create a new character or retry an unconfirmed reply automatically.',
    'receptionOwner.recover': 'Previous computer cannot reconnect',
    'receptionOwner.recoverHint':
      'This disables only this Agent’s old connection. Transfer completes after the server confirms that connection can no longer send messages.',
    'receptionOwner.revoke': 'Disable the old connection and transfer',
    'receptionOwner.cleanupPending':
      'The old connection is being disabled. Transfer is still pending; refresh before continuing.',
    'receptionOwner.limited': 'Showing the 128 most recently active reception bindings.',
    'receptionOwner.return': 'Return to background reception',
    'receptionOwner.stopping': 'Stopping background reception…',
  },
  'zh-CN': {
    'receptionOwner.title': '由谁接待消息',
    'receptionOwner.device': '负责电脑：{{name}}',
    'receptionOwner.active': '后台接待已连接',
    'receptionOwner.idle': '后台已停止，可由你或另一台电脑接手',
    'receptionOwner.draining': '正在等待原电脑停止，并确认已发出的消息',
    'receptionOwner.unreachable': '负责电脑暂时无法连接，接待权尚未转移',
    'receptionOwner.manual': '我要手动接管',
    'receptionOwner.badge.active': '接待中',
    'receptionOwner.badge.idle': '已停止',
    'receptionOwner.badge.draining': '交接中',
    'receptionOwner.badge.unreachable': '联系不上',
    'receptionOwner.move': '换一台电脑接待',
    'receptionOwner.manualHint':
      '等后台显示已停止，再回到 Agent 工具里继续这个任务。处理完后，可交回后台接待。',
    'receptionOwner.choose': '选择接手的电脑',
    'receptionOwner.transfer': '转移接待',
    'receptionOwner.copy': '复制到新电脑的接入指令',
    'receptionOwner.copied': '已复制接入指令',
    'receptionOwner.failed': '操作没有完成，请刷新状态后重试。',
    'receptionOwner.devicesFailed': '无法加载你的电脑列表，请刷新后再转移接待。',
    'receptionOwner.refresh': '刷新接待状态',
    'receptionOwner.pending': '有一条回复尚未确认。接手后会保留原发送编号，先核对是否已送达。',
    'receptionOwner.transferHint':
      '在选定的电脑上，将指令粘贴给一个 Agent 任务，登记该任务后再开启接待。人物身份和消息进度会保留。',
    'receptionOwner.prompt':
      '在这台电脑上接回已有的 Agent Room 人物。执行：{{command}}。使用返回的身份配置，将当前真实宿主任务登记为后台接待，等待用户开启。不要新建人物，也不要自动重试尚未确认的回复。',
    'receptionOwner.recover': '原电脑无法重新连接',
    'receptionOwner.recoverHint':
      '这会停用这个 Agent 在原电脑上的连接。服务器确认旧连接无法再发消息后，才能完成转移。',
    'receptionOwner.revoke': '停用旧连接并转移',
    'receptionOwner.cleanupPending': '正在停用旧连接，接待权尚未转移。请刷新状态后继续。',
    'receptionOwner.limited': '显示最近活动的 128 项接待配置。',
    'receptionOwner.return': '交回后台接待',
    'receptionOwner.stopping': '正在停止后台接待…',
  },
} as const;
