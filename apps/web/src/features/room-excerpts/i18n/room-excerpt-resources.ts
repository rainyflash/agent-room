/**
 * 分享一段对话（specs/redesign/project-rooms.md“分享一段对话”，3P-7b）：对话里每一句的“分享这一段”、选的时候
 * 对话上面那一条、分享的对话框，和房间“分享”对话框里分享过的那一节。
 */
const en = {
  'roomExcerpt.action.share': 'Share this stretch',
  'roomExcerpt.select.message': 'Select this message: {{name}}, {{time}}',
  'roomExcerpt.bar.label': 'Share a stretch of conversation',
  'roomExcerpt.bar.count_one': '{{count}} selected',
  'roomExcerpt.bar.count_other': '{{count}} selected',
  'roomExcerpt.bar.hint': 'Pick a later message to end the stretch there, up to {{max}} messages.',
  'roomExcerpt.bar.tooLong':
    'A stretch can be at most {{max}} messages. Pick a message closer to the start.',
  'roomExcerpt.bar.share': 'Share',
  'roomExcerpt.bar.cancel': 'Cancel',
  'roomExcerpt.dialog.title': 'Share this stretch',
  'roomExcerpt.dialog.subtitle':
    'Anyone with the link can read these messages without signing in. Messages deleted or hidden later stop showing.',
  'roomExcerpt.dialog.creating': 'Making the link…',
  'roomExcerpt.dialog.retry': 'Try again',
  'roomExcerpt.dialog.reselect': 'Change the selection',
  'roomExcerpt.dialog.message': 'A conversation from “{{name}}”',
  'roomExcerpt.dialog.qr.detail':
    'Scan it with a phone camera, WeChat or QQ to open these messages.',
  'roomExcerpt.dialog.qr.label': 'QR code for the link to this stretch',
  'roomExcerpt.dialog.note':
    'What you’ve shared is under Share at the top of the room, where you can delete it any time.',
  'roomExcerpt.preview.withheld': 'A message that isn’t shown publicly',
  'roomExcerpt.preview.attachment': 'Sent a file',
  'roomExcerpt.preview.edited': '(edited)',
  'roomExcerpt.error.title': 'Couldn’t share this stretch',
  'roomExcerpt.error.not_live':
    'This room’s share page isn’t showing a live view right now, so stretches can’t be shared. The owner or an admin can turn it on in room settings, under Public.',
  'roomExcerpt.error.too_long':
    'A stretch can be at most {{max}} messages. Select fewer and share again.',
  'roomExcerpt.error.before_live':
    'Only messages said after the live view was turned on can be shared. Start from a later message.',
  'roomExcerpt.error.range_not_found':
    'These messages can’t be found among the room’s recent messages. They may be too old, or deleted or hidden. Select other messages.',
  'roomExcerpt.error.rate_limited_one':
    'You’ve shared a lot this hour (up to 20 stretches an hour). Try again in {{count}} minute.',
  'roomExcerpt.error.rate_limited_other':
    'You’ve shared a lot this hour (up to 20 stretches an hour). Try again in {{count}} minutes.',
  'roomExcerpt.error.rate_limitedLater':
    'You’ve shared a lot this hour (up to 20 stretches an hour). Try again later.',
  'roomExcerpt.error.not_found': 'This room is gone, or you’re no longer in it.',
  'roomExcerpt.error.forbidden': 'You can’t share from this room right now.',
  'roomExcerpt.error.unavailable': 'It didn’t go through this time. Try again in a moment.',
  'roomExcerpt.error.invalid':
    'Something about this request wasn’t right. Reload the page and try again.',
  'roomExcerpt.error.invalid_origin':
    'This page’s address didn’t check out. Reload the page and try again.',
  'roomExcerpt.error.unreachable': 'Can’t reach Agent Room. Check your connection and try again.',
  'roomExcerpt.error.other': 'It didn’t go through.',
  'roomExcerpt.details': 'Details',
  'roomExcerpt.deleteError.title': 'Couldn’t delete it',
  'roomExcerpt.deleteError.forbidden':
    'Only the person who shared it, the owner and admins can delete it.',
  'roomExcerpt.deleteError.not_found': 'It’s already gone.',
  'roomExcerpt.list.title.mine': 'Stretches you’ve shared',
  'roomExcerpt.list.title.room': 'Shared from this room',
  'roomExcerpt.list.detail.mine':
    'Only you see this list. Anyone with a link can open it until you delete it or the live view is turned off.',
  'roomExcerpt.list.detail.room':
    'As the owner or an admin, you see every stretch shared from this room, but not who shared it. Yours are marked.',
  'roomExcerpt.list.empty':
    'Nothing yet. Point at a message in the conversation and choose Share this stretch.',
  'roomExcerpt.list.loading': 'Loading what’s been shared…',
  'roomExcerpt.list.failed': 'Couldn’t load what’s been shared.',
  'roomExcerpt.list.retry': 'Try again',
  'roomExcerpt.list.item_one': '{{count}} message · {{date}}',
  'roomExcerpt.list.item_other': '{{count}} messages · {{date}}',
  'roomExcerpt.list.mine': 'Yours',
  'roomExcerpt.list.noLink': 'No link right now: this room has no share link.',
  'roomExcerpt.list.copy': 'Copy link',
  'roomExcerpt.list.delete': 'Delete',
  'roomExcerpt.list.confirm': 'Delete it? The link stops working right away.',
  'roomExcerpt.list.confirmYes': 'Delete',
  'roomExcerpt.list.confirmNo': 'Keep it',
  'roomExcerpt.list.deleting': 'Deleting…',
  'roomExcerpt.list.deleted': 'Deleted. The link no longer opens.',
} as const;

export const roomExcerptResources = {
  en,
  'zh-CN': {
    'roomExcerpt.action.share': '分享这一段',
    'roomExcerpt.select.message': '选这一句：{{name}}，{{time}}',
    'roomExcerpt.bar.label': '分享一段对话',
    'roomExcerpt.bar.count_one': '已选 {{count}} 句',
    'roomExcerpt.bar.count_other': '已选 {{count}} 句',
    'roomExcerpt.bar.hint': '再点后面的一句，选到那里为止，最多 {{max}} 句。',
    'roomExcerpt.bar.tooLong': '一段最多 {{max}} 句，选离开头近一点的一句。',
    'roomExcerpt.bar.share': '分享',
    'roomExcerpt.bar.cancel': '取消',
    'roomExcerpt.dialog.title': '分享这一段',
    'roomExcerpt.dialog.subtitle': '拿到链接的人不用登录就看得到这几句；以后撤回、隐藏的不再显示。',
    'roomExcerpt.dialog.creating': '正在生成链接…',
    'roomExcerpt.dialog.retry': '再试一次',
    'roomExcerpt.dialog.reselect': '回去改选',
    'roomExcerpt.dialog.message': '「{{name}}」里的一段对话',
    'roomExcerpt.dialog.qr.detail': '用手机相机、微信或 QQ 扫一扫就能打开这一段。',
    'roomExcerpt.dialog.qr.label': '这一段的链接二维码',
    'roomExcerpt.dialog.note': '分享过的在房间顶上的“分享”里，随时能删。',
    'roomExcerpt.preview.withheld': '一条不公开的消息',
    'roomExcerpt.preview.attachment': '发了一个文件',
    'roomExcerpt.preview.edited': '（改过）',
    'roomExcerpt.error.title': '没分享成',
    'roomExcerpt.error.not_live':
      '这间房的分享页现在没在放实况，不能分享一段对话。房主或管理员可以在房间设置的“公开”里打开。',
    'roomExcerpt.error.too_long': '一段最多 {{max}} 句。少选几句再分享。',
    'roomExcerpt.error.before_live': '只能分享开了实况以后说的话。从后面的一句开始选。',
    'roomExcerpt.error.range_not_found':
      '最近的消息里找不到这几句：可能太早了，或者撤回、隐藏了。换几句再分享。',
    'roomExcerpt.error.rate_limited_one':
      '这一小时分享得太多了（每小时最多 20 段），{{count}} 分钟以后再分享。',
    'roomExcerpt.error.rate_limited_other':
      '这一小时分享得太多了（每小时最多 20 段），{{count}} 分钟以后再分享。',
    'roomExcerpt.error.rate_limitedLater':
      '这一小时分享得太多了（每小时最多 20 段），过一会儿再分享。',
    'roomExcerpt.error.not_found': '这间房不在了，或者你已经不在房间里。',
    'roomExcerpt.error.forbidden': '你现在不能在这间房里分享。',
    'roomExcerpt.error.unavailable': '这次没成，过一会儿再试。',
    'roomExcerpt.error.invalid': '请求有点不对，刷新页面再试。',
    'roomExcerpt.error.invalid_origin': '这个页面的来源对不上，刷新页面再试。',
    'roomExcerpt.error.unreachable': '连不上 Agent Room，检查网络后再试。',
    'roomExcerpt.error.other': '没成。',
    'roomExcerpt.details': '详情',
    'roomExcerpt.deleteError.title': '没删掉',
    'roomExcerpt.deleteError.forbidden': '只有分享的人、房主和管理员能删。',
    'roomExcerpt.deleteError.not_found': '这一段已经不在了。',
    'roomExcerpt.list.title.mine': '我分享过的',
    'roomExcerpt.list.title.room': '这间房分享过的',
    'roomExcerpt.list.detail.mine':
      '只有你看得到这些。拿到链接的人都打得开，直到你删掉，或者实况关掉。',
    'roomExcerpt.list.detail.room':
      '你是房主或管理员，看得到这间房分享出去的每一段，但看不出是谁分享的；标着“你分享的”是你自己的。',
    'roomExcerpt.list.empty': '还没有。在对话里指着一句，点“分享这一段”。',
    'roomExcerpt.list.loading': '正在读分享过的…',
    'roomExcerpt.list.failed': '没读出来分享过的。',
    'roomExcerpt.list.retry': '再试一次',
    'roomExcerpt.list.item_one': '{{count}} 句 · {{date}}',
    'roomExcerpt.list.item_other': '{{count}} 句 · {{date}}',
    'roomExcerpt.list.mine': '你分享的',
    'roomExcerpt.list.noLink': '现在没有链接：这间房没有分享链接。',
    'roomExcerpt.list.copy': '复制链接',
    'roomExcerpt.list.delete': '删掉',
    'roomExcerpt.list.confirm': '删掉这一段？链接马上就打不开了。',
    'roomExcerpt.list.confirmYes': '删掉',
    'roomExcerpt.list.confirmNo': '不删了',
    'roomExcerpt.list.deleting': '正在删…',
    'roomExcerpt.list.deleted': '删掉了，链接打不开了。',
  } satisfies Record<keyof typeof en, string>,
} as const;
