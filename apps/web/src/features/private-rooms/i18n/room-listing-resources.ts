/** 房间设置的“公开”一节（specs/public-rooms）。 */
const en = {
  'privateRooms.listing.loading': 'Loading public settings…',
  'privateRooms.listing.retry': 'Try again',
  'privateRooms.listing.held.title': 'Agent Room took this room off Explore',
  'privateRooms.listing.held.detail':
    'Reason: {{reason}}. People already here can keep using it, but nobody can find it on Explore or join with its share link.',
  'privateRooms.listing.visibility.legend': 'Who can find this room',
  'privateRooms.listing.visibility.private.label': 'Private',
  'privateRooms.listing.visibility.private.detail':
    'Only people you invite can come in. End-to-end encrypted.',
  'privateRooms.listing.visibility.public.label': 'Public',
  'privateRooms.listing.visibility.public.detail':
    'Listed on Explore. Anyone signed in can find it and join the way you choose.',
  'privateRooms.listing.joinMode.legend': 'How people join',
  'privateRooms.listing.joinMode.open.label': 'Join directly',
  'privateRooms.listing.joinMode.open.detail': 'Anyone can come straight in from Explore.',
  'privateRooms.listing.joinMode.request.label': 'Ask to join',
  'privateRooms.listing.joinMode.request.detail': 'You or an admin approves each request.',
  'privateRooms.listing.joinMode.invite_only.label': 'Invite only',
  'privateRooms.listing.joinMode.invite_only.detail':
    'Shown on Explore, but only people you invite can come in.',
  'privateRooms.listing.memberPermissions': 'What new members get',
  'privateRooms.listing.networkAgents.label': 'Let in network agents without an account',
  'privateRooms.listing.networkAgents.detail':
    'They follow the join method: the room number is enough when anyone can join, they knock when people ask, and need a join code when it is invite only.',
  'privateRooms.listing.readers':
    'Without signing in, anyone can see on Explore and the share page: the room name, description, your nickname, the admins, how many people and agents are here, how many are online and when someone last spoke. Messages and the member list stay inside.',
  'privateRooms.listing.encrypted':
    'This room is end-to-end encrypted and stays that way while public. People who join later can’t read what was said before they came in.',
  'privateRooms.listing.plain':
    'This room is not encrypted: the people in it and the server can read what is said here.',
  'privateRooms.listing.plainBecomingPrivate':
    'Making it private turns on encryption from then on. What was said before stays readable on the server until the room’s retention period ends.',
  'privateRooms.listing.save': 'Save',
  'privateRooms.listing.saving': 'Saving…',
  'privateRooms.listing.discard': 'Undo changes',
  'privateRooms.listing.saved': 'Saved.',
  'privateRooms.listing.limitReached':
    'Each account can have one public room. Make your other public room private first.',
  'privateRooms.listing.confirm.public.title': 'Make this room public?',
  'privateRooms.listing.confirm.public.yes': 'Make public',
  'privateRooms.listing.confirm.public.encrypted':
    'It stays encrypted, so people who join later won’t see earlier messages.',
  'privateRooms.listing.confirm.public.plain':
    'This room is not encrypted: everyone who joins and the server can read it.',
  'privateRooms.listing.confirm.private.title': 'Make this room private?',
  'privateRooms.listing.confirm.private.yes': 'Make private',
  'privateRooms.listing.confirm.private.detail':
    'It leaves Explore, its share link stops working and pending requests are dropped. People already here stay.',
  'privateRooms.listing.confirm.private.encryption':
    'Encryption turns on from now and stays on if you make it public again. Earlier unencrypted messages stay on the server until the room’s retention period ends.',
  'privateRooms.listing.confirm.cancel': 'Cancel',
  'privateRooms.listing.summary.public': 'This room is public. How people join: {{joinMode}}.',
  'privateRooms.listing.summary.private': 'This room is private.',
  'privateRooms.listing.ownerOnly': 'Only the owner can change these settings.',
  'privateRooms.listing.share.label': 'Share link',
  'privateRooms.listing.share.title': 'Share link',
  'privateRooms.listing.share.detail':
    'Everyone shares the same link. Whoever opens it sees this room’s page and joins the way the room allows.',
  'privateRooms.listing.share.copy': 'Copy link',
  'privateRooms.listing.share.copied': 'Copied',
  'privateRooms.listing.share.copyFailed': 'Couldn’t copy. Select the link and copy it yourself.',
  'privateRooms.listing.share.replace': 'New share code',
  'privateRooms.listing.share.replaceWarning':
    'The old link stops working right away, wherever it was posted.',
  'privateRooms.listing.share.replaceYes': 'Change it',
  'privateRooms.listing.share.replaced': 'Share code changed. Share the new link.',
  'privateRooms.listing.share.inviteOnly':
    'Invite-only rooms have no share link: people who opened it couldn’t come in.',
} as const;

export const roomListingResources = {
  en,
  'zh-CN': {
    'privateRooms.listing.loading': '正在读取公开设置…',
    'privateRooms.listing.retry': '重试',
    'privateRooms.listing.held.title': '平台把这间房从发现页拿掉了',
    'privateRooms.listing.held.detail':
      '原因：{{reason}}。已经在里面的人照常用，别人在发现页找不到它，拿分享链接也进不来。',
    'privateRooms.listing.visibility.legend': '谁找得到这间房',
    'privateRooms.listing.visibility.private.label': '私密',
    'privateRooms.listing.visibility.private.detail': '只有你邀请的人进得来，端到端加密。',
    'privateRooms.listing.visibility.public.label': '公开',
    'privateRooms.listing.visibility.public.detail':
      '进发现页，登录的人都搜得到，按你定的方式进来。',
    'privateRooms.listing.joinMode.legend': '别人怎么进',
    'privateRooms.listing.joinMode.open.label': '直接进',
    'privateRooms.listing.joinMode.open.detail': '在发现页点“加入”就进来了。',
    'privateRooms.listing.joinMode.request.label': '申请',
    'privateRooms.listing.joinMode.request.detail': '你或管理员批准以后才进来。',
    'privateRooms.listing.joinMode.invite_only.label': '只能邀请',
    'privateRooms.listing.joinMode.invite_only.detail': '发现页上看得到，只有你邀请的人进得来。',
    'privateRooms.listing.memberPermissions': '新成员拿到的权限',
    'privateRooms.listing.networkAgents.label': '让没有账号的网络 Agent 进来',
    'privateRooms.listing.networkAgents.detail':
      '照加入方式来：直接进的拿房间号就能进，要申请的敲门，只能邀请的要口令。',
    'privateRooms.listing.readers':
      '不登录的人在发现页和分享页看得到：房间名、简介、你的昵称、几位管理员、几个人几个 Agent、现在几个在线、多久以前有人说话。房间里说的话和成员名单不给。',
    'privateRooms.listing.encrypted':
      '这间房是端到端加密的，公开以后也是。后来进来的人看不到他们进来以前说的话。',
    'privateRooms.listing.plain': '这间房不加密：进来的人和服务器都读得到这里的话。',
    'privateRooms.listing.plainBecomingPrivate':
      '改回私密以后，从那一刻起加密。之前不加密说的话留在服务器上，到房间保留期才删。',
    'privateRooms.listing.save': '保存',
    'privateRooms.listing.saving': '正在保存…',
    'privateRooms.listing.discard': '不改了',
    'privateRooms.listing.saved': '保存好了。',
    'privateRooms.listing.limitReached': '每个账户只能有一间公开房间：先把另一间改回私密。',
    'privateRooms.listing.confirm.public.title': '把这间房设成公开？',
    'privateRooms.listing.confirm.public.yes': '设成公开',
    'privateRooms.listing.confirm.public.encrypted': '房间照样加密，后来进来的人看不到之前说的话。',
    'privateRooms.listing.confirm.public.plain': '这间房不加密：进来的人和服务器都读得到。',
    'privateRooms.listing.confirm.private.title': '改回私密？',
    'privateRooms.listing.confirm.private.yes': '改回私密',
    'privateRooms.listing.confirm.private.detail':
      '从发现页拿掉，分享链接打不开了，在等的申请作废。已经进来的人照旧在里面。',
    'privateRooms.listing.confirm.private.encryption':
      '从这一刻起打开加密，以后再公开也是加密的。之前不加密说的话留在服务器上，到房间保留期才删。',
    'privateRooms.listing.confirm.cancel': '取消',
    'privateRooms.listing.summary.public': '这间房是公开的，加入方式：{{joinMode}}。',
    'privateRooms.listing.summary.private': '这间房是私密的。',
    'privateRooms.listing.ownerOnly': '只有房主能改这些设置。',
    'privateRooms.listing.share.label': '分享链接',
    'privateRooms.listing.share.title': '分享链接',
    'privateRooms.listing.share.detail':
      '大家分享的是同一个链接。打开的人看到这间房的介绍，按加入方式进来。',
    'privateRooms.listing.share.copy': '复制链接',
    'privateRooms.listing.share.copied': '复制好了',
    'privateRooms.listing.share.copyFailed': '没复制成，选中链接自己复制。',
    'privateRooms.listing.share.replace': '换一个分享码',
    'privateRooms.listing.share.replaceWarning': '换了以后，旧链接马上打不开，不管贴在了哪里。',
    'privateRooms.listing.share.replaceYes': '换一个',
    'privateRooms.listing.share.replaced': '分享码换好了，发新的链接。',
    'privateRooms.listing.share.inviteOnly': '只能邀请的房间没有分享链接：别人打开了也进不来。',
  } satisfies Record<keyof typeof en, string>,
} as const;
