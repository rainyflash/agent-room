export const receptionResources = {
  en: {
    'reception.agentDescription': 'Background replies for this agent in this room.',
    'reception.agentUnavailable':
      'This agent has no background task registered here. Send it the registration request above. For an agent on another computer, manage it there.',
    'reception.starting': 'Starting background replies…',
    'reception.reconnecting': 'Reconnecting; replies are paused',
    'reception.enable': 'Enable background replies',
    'reception.enableDescription':
      'It wakes when you talk to it in this room, or when someone mentions or replies to it in a private room; messages in a row become one reply, and it may decide not to reply. Replies are visible to all room members, including people who join later. New authorizations last {{days}} days, up to 10 replies/minute and 1,000 replies total. Keep the computer and Agent Room running.',
    'reception.manual': 'Use an existing authorization or custom executable',
    'reception.title': 'Background replies',
    'reception.description':
      'Keep a registered agent task ready to reply when you talk to it. Keep Agent Room running, and pause background replies before using the task yourself.',
    'reception.empty':
      'No agent task has background replies yet. Ask an agent task to register, then choose a room grant below.',
    'reception.prompt':
      'Register this exact task for Agent Room background replies. If connected by CLI, use the same installed executable and --profile value to run register with the actual --workspace; register --help lists the supported tools and how the task is recognized. If connected by MCP, use agent_room_register_reception with this task’s sessionId, accurate hostType and workspace. Never guess a task ID or select the latest task. Registration alone must not enable replies.',
    'reception.copy': 'Copy registration request',
    'reception.copied': 'Copied',
    'reception.copyFailed': 'Could not copy. Select the request text and copy it manually.',
    'reception.loading': 'Loading background replies…',
    'reception.failed': 'Could not load background replies. Refresh to try again.',
    'reception.failure.host':
      'The agent program did not reply. Check that it is installed, signed in and has quota, then start the task again.',
    'reception.failure.files':
      'Local files for this task are missing or unreadable. Remove the task and set it up again.',
    'reception.failure.authorization':
      'The reply authorization is no longer valid. Enable background replies again to create a new one.',
    'reception.failure.session':
      'This agent session is no longer available. Register the task again from the agent.',
    'reception.failure.review': 'A reply is waiting for your review before the task can continue.',
    'reception.failure.unresponsive':
      'The task did not respond in time. Stop it and start it again.',
    'reception.failure.unknown':
      'The last try failed on this computer. Open the log folder from Settings → This computer for details.',
    'reception.refresh': 'Refresh',
    'reception.showPrompt': 'Show the request text',
    'reception.task': 'Registered task',
    'reception.grant': 'Reply authorization',
    'reception.choose': 'Choose an authorization',
    'reception.grantLabel': 'Until {{time}} · {{limit}} replies/min',
    'reception.noGrant':
      'This agent has no reply authorization for this room yet. Create one in Room settings → Automation.',
    'reception.bind': 'Add background reply task',
    'reception.start': 'Start',
    'reception.pause': 'Pause',
    'reception.remove': 'Remove',
    'reception.update': 'Save authorization and paths',
    'reception.settings': 'Authorization and paths',
    'reception.executable': 'Program path (found automatically if empty)',
    'reception.workspace': 'Workspace',
    'reception.received': 'Message received',
    'reception.running': 'Replying',
    'reception.verifying': 'Checking that the reply reached the room',
    'reception.replied': 'The reply is in the room',
    'reception.needs_review': 'Needs attention',
    'reception.skipped': 'Skipped by owner',
    'reception.no_reply': 'Read the messages; no reply needed',
    'reception.waiting': 'Waiting for messages for it',
    'reception.paused': 'Paused',
    'reception.pending':
      'The last reply is not confirmed yet. “Check” only looks at the room and does not wake the task; “Reply again” will not post twice.',
    'reception.verify': 'Check',
    'reception.retry': 'Reply again',
    'reception.skip': 'Skip these messages',
    'reception.legacyPending':
      'This came from an older version and cannot be confirmed automatically. Look at the room and the task before skipping; retrying automatically could reply twice.',
    'reception.login': 'Sign in to choose your reply authorization.',
  },
  'zh-CN': {
    'reception.agentDescription': '管理这个 Agent 在本房间的后台回复。',
    'reception.agentUnavailable':
      '这个 Agent 尚未在本机登记后台任务。可以把上面的登记请求发给它；在其他电脑运行的 Agent，需要到那台电脑上管理。',
    'reception.starting': '正在开启后台回复…',
    'reception.reconnecting': '正在恢复连接，回复暂时中断',
    'reception.enable': '开启后台回复',
    'reception.enableDescription':
      '你在本房间跟它说话，或者私人房间里有人点名、回复它时，它会醒来；连着的几条合成一次回复，它也可能觉得不用回。回复对房间内所有成员（包括之后加入的人）可见。新授权有效期 {{days}} 天，每分钟最多 10 条、累计最多 1,000 条。电脑和 Agent Room 需要保持运行。',
    'reception.manual': '使用已有授权或指定程序',
    'reception.title': '后台回复',
    'reception.description':
      '让登记过的 Agent 任务在你跟它说话时随时回复。电脑和 Agent Room 需要保持运行；自己手动用这个任务前，请先暂停后台回复。',
    'reception.empty': '还没有 Agent 任务开着后台回复。先让 Agent 任务登记，再在下面选择房间授权。',
    'reception.prompt':
      '请登记当前任务，以便 Agent Room 提供后台回复。通过 CLI 接入时，复用已安装程序和本任务的 --profile 执行 register，带上真实 --workspace；支持哪些工具、怎么识别任务，看 register --help。通过 MCP 接入时，使用 agent_room_register_reception，带上本任务 sessionId、准确 hostType 和工作目录。不能猜测任务 ID 或选择最近任务。登记本身不启用自动回复。',
    'reception.copy': '复制登记请求',
    'reception.copied': '已复制',
    'reception.copyFailed': '复制失败，请选中请求文字手动复制。',
    'reception.loading': '正在读取后台回复…',
    'reception.failed': '暂时读不到后台回复的信息，请刷新重试。',
    'reception.failure.host':
      'Agent 程序没有给出回复。确认它已安装、已登录、还有额度，再重新启动任务。',
    'reception.failure.files': '这个任务的本机文件缺失或读不出来。移除任务后重新登记。',
    'reception.failure.authorization': '回复授权已失效。重新启用后台回复会创建新的授权。',
    'reception.failure.session': '这个 Agent 会话已不可用。请从 Agent 里重新登记任务。',
    'reception.failure.review': '有一条回复在等你审阅，任务才能继续。',
    'reception.failure.unresponsive': '任务没有及时响应。停止后再启动一次。',
    'reception.failure.unknown':
      '上次尝试在这台电脑上失败了。去“设置 → 这台电脑”打开日志文件夹看看。',
    'reception.refresh': '刷新',
    'reception.showPrompt': '查看登记请求原文',
    'reception.task': '已登记的任务',
    'reception.grant': '回复授权',
    'reception.choose': '选择授权',
    'reception.grantLabel': '有效至 {{time}} · 每分钟 {{limit}} 条',
    'reception.noGrant':
      '这个 Agent 在这个房间还没有可用的回复授权。请在“房间设置 → 自动发言”里创建。',
    'reception.bind': '添加后台回复任务',
    'reception.start': '开启',
    'reception.pause': '暂停',
    'reception.remove': '移除',
    'reception.update': '保存授权与路径',
    'reception.settings': '授权与路径',
    'reception.executable': '程序路径（留空会自动找）',
    'reception.workspace': '工作目录',
    'reception.received': '已收到消息',
    'reception.running': '正在回复',
    'reception.verifying': '正在确认回复已发到房间',
    'reception.replied': '回复已发到房间',
    'reception.needs_review': '需要处理',
    'reception.skipped': '已由你跳过',
    'reception.no_reply': '看过了，觉得不用回',
    'reception.waiting': '正在等跟它有关的消息',
    'reception.paused': '已暂停',
    'reception.pending':
      '上一条回复还没确认发出去。“确认一下”只看房间，不会叫醒任务；“重新回复”不会重复发。',
    'reception.verify': '确认一下',
    'reception.retry': '重新回复',
    'reception.skip': '跳过这几条消息',
    'reception.legacyPending':
      '这条是旧版本发的，没法自动确认。先看看房间和任务再决定要不要跳过；自动重试可能会回复两次。',
    'reception.login': '请先登录，再选择你的回复授权。',
  },
} as const;
