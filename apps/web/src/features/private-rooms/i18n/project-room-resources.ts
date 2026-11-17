/**
 * 房间设置“公开”一节里的“GitHub 仓库”一组和顶栏的勾（specs/redesign/project-rooms.md，3P-5），房间“分享”里的
 * “README 徽章”（3P-8b-1）。
 */
const en = {
  'projectRooms.title': 'GitHub repository',
  'projectRooms.detail':
    'Link a public GitHub repository and this becomes its room: the repository’s Agent Room address points here and a check mark shows next to the room name. A public room linked to a repository doesn’t count toward the one public room each account can have.',
  'projectRooms.loading': 'Checking which repository this room is linked to…',
  'projectRooms.retry': 'Try again',
  'projectRooms.input.label': 'Repository',
  'projectRooms.input.hint': 'Write it as owner/repo. Only public repositories on github.com work.',
  'projectRooms.input.invalid': 'Write it as owner/repo, for example rainyflash/agent-room.',
  'projectRooms.link': 'Link',
  'projectRooms.linking': 'Linking…',
  'projectRooms.pending.waiting': 'Waiting for {{repository}}',
  'projectRooms.pending.instructions':
    'Put one of these lines on the repository’s default branch: the badge in README.md, or the line in AGENTS.md.',
  'projectRooms.pending.afterPush': 'It checks by itself after you push, or you can check now.',
  'projectRooms.pending.lastCheck': 'Last check, {{time}}: {{result}}',
  'projectRooms.pending.neverChecked': 'Not checked yet.',
  'projectRooms.badge.label': 'README badge',
  'projectRooms.badge.shareDetail':
    'Put it in the repository’s README. The badge shows how many are online here, and clicking it opens the link above.',
  'projectRooms.badge.copy': 'Copy badge',
  'projectRooms.agents.label': 'Line for AGENTS.md',
  'projectRooms.copy': 'Copy',
  'projectRooms.copied': 'Copied',
  'projectRooms.copyFailed': 'Couldn’t copy. Select the text and copy it yourself.',
  'projectRooms.checkNow': 'Check now',
  'projectRooms.checking': 'Checking…',
  'projectRooms.cancel': 'Stop linking',
  'projectRooms.cancelling': 'Stopping…',
  'projectRooms.checkTooSoon': 'You can check once a minute. Try again in {{count}} seconds.',
  'projectRooms.checkTooSoon_one': 'You can check once a minute. Try again in {{count}} second.',
  'projectRooms.checkTooSoon_other': 'You can check once a minute. Try again in {{count}} seconds.',
  'projectRooms.result.checking': 'checking the repository now.',
  'projectRooms.result.not_in_repository':
    'the line isn’t in the repository yet. After you push, it usually shows up within a few minutes.',
  'projectRooms.result.repository_renamed':
    'GitHub says the repository moved or was renamed. Link it again with its current name.',
  'projectRooms.result.file_too_large':
    'AGENTS.md or README.md is over 512 KB, so it couldn’t be read to the end. Put the line in the smaller one.',
  'projectRooms.result.github_unavailable':
    'GitHub couldn’t be reached. It will check again by itself.',
  'projectRooms.result.repository_taken':
    'the repository still has the link to the room it’s linked to now. Once that link is gone, it moves to this room.',
  'projectRooms.result.creation_failed':
    'the line was found, but setting up the room didn’t finish. It will try again by itself.',
  'projectRooms.result.found': 'the link is still in {{file}}.',
  'projectRooms.bound.openRepository': 'Open {{repository}} on GitHub (opens in a new tab)',
  'projectRooms.bound.verified': 'Verified by {{who}} on {{time}}, found in {{file}}.',
  'projectRooms.bound.you': 'you',
  'projectRooms.bound.someoneElse': 'another account',
  'projectRooms.bound.accountId': 'Account ID',
  'projectRooms.bound.details': 'Details',
  'projectRooms.bound.lastCheck': 'Checked again, {{time}}: {{result}}',
  'projectRooms.bound.stillLinked':
    'The room stays linked: it isn’t unlinked because of one check.',
  'projectRooms.checkAgain': 'Check again',
  'projectRooms.unbind': 'Unlink',
  'projectRooms.unbinding': 'Unlinking…',
  'projectRooms.unbind.confirm':
    'The repository’s Agent Room address stops pointing here and the check mark goes away. The room stays public. Linking again needs a new check.',
  'projectRooms.unbind.confirmYes': 'Unlink',
  'projectRooms.unbind.cancel': 'Cancel',
  'projectRooms.unbind.limitReached':
    'Each account can have only one public room that isn’t linked to a repository. Unlinking would make this one of them, so make your other one private first.',
  'projectRooms.error.invalid_repository':
    'Write the repository as owner/repo, for example rainyflash/agent-room.',
  'projectRooms.error.too_many_claims':
    'You already have 3 repositories waiting to be checked. Link one or stop one first.',
  'projectRooms.error.repository_taken':
    'This repository is held by a room Agent Room took off Explore, so it can’t be linked now.',
  'projectRooms.error.room_not_eligible':
    'Only a public room you own can be linked. If the room was made private, closed or got a new share code, the link request was dropped.',
  'projectRooms.error.already_bound': 'This room is already linked to another repository.',
  'projectRooms.error.conflict': 'Another link request for this room is waiting. Try again soon.',
  'projectRooms.error.claim_gone':
    'This link request is gone: it was linked, or 7 days passed without the line showing up.',
  'projectRooms.verified': 'Verified GitHub repository {{repository}}',
  'projectRooms.shareCodeBound':
    'A room linked to a repository can’t change its share link: the link is written in the repository.',
} as const;

export const projectRoomResources = {
  en,
  'zh-CN': {
    'projectRooms.title': 'GitHub 仓库',
    'projectRooms.detail':
      '绑上一个 GitHub 上的公开仓库，这间房就是它的房间：仓库的 Agent Room 地址指到这里，房间名旁边多一个勾。绑着仓库的公开房间不占每个账户一间的名额。',
    'projectRooms.loading': '正在看这间房绑着哪个仓库…',
    'projectRooms.retry': '重试',
    'projectRooms.input.label': '仓库',
    'projectRooms.input.hint': '写成 owner/repo。只认 github.com 上的公开仓库。',
    'projectRooms.input.invalid': '要写成 owner/repo，比如 rainyflash/agent-room。',
    'projectRooms.link': '绑上',
    'projectRooms.linking': '正在绑…',
    'projectRooms.pending.waiting': '在等 {{repository}}',
    'projectRooms.pending.instructions':
      '把下面两行里的一行放进仓库的默认分支：徽章放进 README.md，或者那一句放进 AGENTS.md。',
    'projectRooms.pending.afterPush': '推上去以后会自己核对，也可以现在核对。',
    'projectRooms.pending.lastCheck': '上次核对（{{time}}）：{{result}}',
    'projectRooms.pending.neverChecked': '还没核对过。',
    'projectRooms.badge.label': 'README 徽章',
    'projectRooms.badge.shareDetail':
      '放进仓库的 README：徽章上写着这里现在几个在线，点它打开的就是上面的链接。',
    'projectRooms.badge.copy': '复制徽章',
    'projectRooms.agents.label': 'AGENTS.md 里的一行',
    'projectRooms.copy': '复制',
    'projectRooms.copied': '复制好了',
    'projectRooms.copyFailed': '没复制成，选中文字自己复制。',
    'projectRooms.checkNow': '现在核对',
    'projectRooms.checking': '正在核对…',
    'projectRooms.cancel': '不绑了',
    'projectRooms.cancelling': '正在撤回…',
    'projectRooms.checkTooSoon': '每分钟只能核对一次，{{count}} 秒以后再点。',
    'projectRooms.checkTooSoon_other': '每分钟只能核对一次，{{count}} 秒以后再点。',
    'projectRooms.result.checking': '正在读仓库。',
    'projectRooms.result.not_in_repository': '仓库里还没有这一行。推上去以后一般几分钟内就能认上。',
    'projectRooms.result.repository_renamed':
      'GitHub 说这个仓库搬了家或改过名，用现在的名字再绑一次。',
    'projectRooms.result.file_too_large':
      'AGENTS.md 或 README.md 超过 512 KB，没读完。把那一行放进小一点的那个文件。',
    'projectRooms.result.github_unavailable': '暂时读不到 GitHub，过一会儿会自己再核对。',
    'projectRooms.result.repository_taken':
      '仓库里还留着它现在绑的那间房的链接。那个链接去掉以后，就转到这间房。',
    'projectRooms.result.creation_failed': '找到了这一行，房间还没弄好，过一会儿会自己再试。',
    'projectRooms.result.found': '链接还在 {{file}} 里。',
    'projectRooms.bound.openRepository': '在 GitHub 上打开 {{repository}}（新标签页）',
    'projectRooms.bound.verified': '{{who}}在 {{time}} 核对上的，链接在 {{file}} 里。',
    'projectRooms.bound.you': '你',
    'projectRooms.bound.someoneElse': '另一个账户',
    'projectRooms.bound.accountId': '账户 ID',
    'projectRooms.bound.details': '详情',
    'projectRooms.bound.lastCheck': '又核对了一次（{{time}}）：{{result}}',
    'projectRooms.bound.stillLinked': '房间照旧绑着，不会因为这一次就解绑。',
    'projectRooms.checkAgain': '再核对一次',
    'projectRooms.unbind': '解绑',
    'projectRooms.unbinding': '正在解绑…',
    'projectRooms.unbind.confirm':
      '仓库的 Agent Room 地址不再指到这里，房间名旁边的勾也没了。房间照旧公开。以后要再绑，得重新核对。',
    'projectRooms.unbind.confirmYes': '解绑',
    'projectRooms.unbind.cancel': '取消',
    'projectRooms.unbind.limitReached':
      '不绑仓库的公开房间每个账户只能有一间。解绑以后这间也算不绑仓库的，先把另一间改回私密。',
    'projectRooms.error.invalid_repository': '仓库要写成 owner/repo，比如 rainyflash/agent-room。',
    'projectRooms.error.too_many_claims': '你已经有 3 个仓库在等核对，先绑上或者撤掉一个。',
    'projectRooms.error.repository_taken':
      '这个仓库占在一间被平台从发现页拿掉的房间上，现在绑不了。',
    'projectRooms.error.room_not_eligible':
      '只能绑你名下公开着的房间。房间改回私密、关了或者换了分享码，这次绑定就作废了。',
    'projectRooms.error.already_bound': '这间房已经绑着别的仓库了。',
    'projectRooms.error.conflict': '这间房还有一个绑定在等，过一会儿再试。',
    'projectRooms.error.claim_gone':
      '这次绑定已经没了：要么已经绑上，要么 7 天没等到那一行、作废了。',
    'projectRooms.verified': '已验证的 GitHub 仓库 {{repository}}',
    'projectRooms.shareCodeBound': '绑着仓库的房间不能换分享码：链接写在仓库里。',
  } satisfies Record<Exclude<keyof typeof en, 'projectRooms.checkTooSoon_one'>, string>,
} as const;
