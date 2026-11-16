/** 房间设置“公开”一节里的“分享页放实况”一组（specs/redesign/project-rooms.md，3P-6b）。 */
const en = {
  'roomLive.title': 'Live view on the share page',
  'roomLive.detail':
    'When it’s on, people who aren’t signed in can see what’s said here from then on, on the share page. Encrypted rooms and rooms people have to ask to join or be invited to can’t turn it on.',
  'roomLive.loading': 'Checking the live view…',
  'roomLive.retry': 'Try again',
  'roomLive.switch': 'Show what’s said here on the share page',
  'roomLive.state.live': 'On since {{time}}. Only what’s said after that is shown.',
  'roomLive.state.paused': 'On, but not shown on the share page right now.',
  'roomLive.state.off': 'Off.',
  'roomLive.saving': 'Saving…',
  'roomLive.managersOnly': 'Only the owner and admins can turn it on or off.',
  'roomLive.condition.sub_room': 'Only the main room can show a live view. Sub-rooms can’t.',
  'roomLive.condition.closed': 'This room is archived, so it can’t show a live view.',
  'roomLive.condition.not_listed': 'This room is private. Make it public first.',
  'roomLive.condition.platform_hold':
    'Agent Room took this room off Explore, so the live view is paused.',
  'roomLive.condition.encrypted':
    'This room is end-to-end encrypted, and encrypted rooms can’t show a live view.',
  'roomLive.condition.join_mode':
    'Only rooms people can join directly can show a live view. Set How people join to Join directly first.',
  'roomLive.confirm.title': 'Show what’s said here on the share page?',
  'roomLive.confirm.detail':
    'From now on, anyone can see what’s said in this room on its share page without signing in: the latest 20 messages, with the names of the people and agents who said them and when. Earlier messages and sub-rooms aren’t shown. It turns off by itself if the room is made private or people can no longer join directly.',
  'roomLive.confirm.yes': 'Turn on',
  'roomLive.confirm.cancel': 'Cancel',
  'roomLive.details': 'Details',
  'roomLive.error.forbidden': 'Only the owner and admins can turn the live view on or off.',
  'roomLive.error.not_found': 'This room is gone, or you’re no longer in it.',
  'roomLive.error.not_eligible': 'This room can’t show a live view right now.',
  'roomLive.error.conflict':
    'The room’s settings just changed. Look at them again, then turn it on again.',
  'roomLive.error.dependency_unavailable':
    'The live view couldn’t be changed right now. Try again in a moment.',
  'roomLive.error.internal': 'Something went wrong on the server. Try again later.',
  'roomLive.error.invalid': 'Agent Room didn’t accept this request. Reload the page and try again.',
  'roomLive.error.invalid_origin':
    'This request didn’t come from an Agent Room page. Reload Agent Room and try again.',
} as const;

export const roomLiveViewResources = {
  en,
  'zh-CN': {
    'roomLive.title': '分享页放实况',
    'roomLive.detail':
      '打开以后，不登录的人在分享页上看得到这里以后说的话；加密、要申请、只能邀请的房间不能开。',
    'roomLive.loading': '正在看实况开没开…',
    'roomLive.retry': '重试',
    'roomLive.switch': '在分享页上放这里说的话',
    'roomLive.state.live': '{{time}} 起开着，只放那以后说的话。',
    'roomLive.state.paused': '开着，但分享页上现在没在放。',
    'roomLive.state.off': '关着。',
    'roomLive.saving': '正在保存…',
    'roomLive.managersOnly': '只有房主和管理员能开关。',
    'roomLive.condition.sub_room': '只有主房间能放实况，子房间不能。',
    'roomLive.condition.closed': '房间已经归档了，不能放实况。',
    'roomLive.condition.not_listed': '房间不公开，先设成公开。',
    'roomLive.condition.platform_hold': '这间房被平台从发现页拿掉了，实况停着。',
    'roomLive.condition.encrypted': '这间房是端到端加密的，加密的房间不能放实况。',
    'roomLive.condition.join_mode': '只有直接进的房间能放实况。先把“别人怎么进”改成“直接进”。',
    'roomLive.confirm.title': '在分享页上放这里说的话？',
    'roomLive.confirm.detail':
      '打开以后，谁都能在分享页上看到这间房从现在起说的话，不用登录：最近 20 句，说话的人和 Agent 的名字、时间。之前说的和子房间不放。改回私密、改成要申请或只能邀请时会自己关掉。',
    'roomLive.confirm.yes': '打开',
    'roomLive.confirm.cancel': '取消',
    'roomLive.details': '详情',
    'roomLive.error.forbidden': '只有房主和管理员能开关实况。',
    'roomLive.error.not_found': '没有这个房间，或者你已经不在里面了。',
    'roomLive.error.not_eligible': '这间房现在不能放实况。',
    'roomLive.error.conflict': '房间的设置刚被改过，看一眼再开。',
    'roomLive.error.dependency_unavailable': '实况暂时没开关成，过一会儿再试。',
    'roomLive.error.internal': '服务器出了错，过一会儿再试。',
    'roomLive.error.invalid': '这次请求没被接受，刷新页面再试。',
    'roomLive.error.invalid_origin':
      '请求不是从 Agent Room 的页面发出的，重新打开 Agent Room 再试。',
  } satisfies Record<keyof typeof en, string>,
} as const;
