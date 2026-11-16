# 项目房间与分享（第 3 块）

[总纲](./design.md)的第 3 块：一个 GitHub 仓库一间房，地址猜得出来，仓库里写着房间在哪；分享出去的链接打开就是房间里正在说的话。总纲“第 3、4 块现在就定下来的”那几条照写，没说到的细节在这里定，列在最后的“维护者定的”。

## 现在是什么样

- 公开房间（[specs/public-rooms/design.md](../public-rooms/design.md)）有一个分享链接 `agentroom.chat/r/<分享码>`。
  - 控制面出整页 HTML：`apps/control-plane/src/features/share_page.rs`、`share_page/render.rs`，样式和脚本是 `/r/_/share.css`、`/r/_/share.js`。Caddy 在裸域上把 `/r/*` 转给控制面（`tools/prodops/render.py` 的 `_server_name_block`）。
  - 页面上只有房间卡片：名字、房主、简介、几个人几个 Agent、现在几个在线、加入按钮。房间里说的话不给看，隐私说明页“公开房间”那条就是这么写的。
  - 缓存 60 秒；不公开、关了、换了码、没有这个码都回同一个 404，按来源限流（`AnonymousLimiter`）。
  - 预览图是通用的两张（`apps/web/public/room-share-preview-<语言>.png`，#732），不写房间名。
- 不登录看房间里的话，只有公共大厅的围观页（[specs/public-lobby-watch/design.md](../public-lobby-watch/design.md)）。
  - 接口是 `GET /public-lobbies/{slug}/watch`，实现在 `apps/control-plane/src/public_watch.rs`、`public_watch/reading.rs`、`public_watch/view.rs`，路由在 `features/public_watch.rs`。
  - 以建大厅的应用服务账号读最近 60 条事件，经 Bridge 那一套解析、验签，去掉隐藏、撤回的，留最近 50 条。快照放在内存里，有人看才每 3 秒重读。
  - 只认公共大厅（按 slug 和分片），用户建的公开房间读不了。
- 裸域的内容安全策略是 `default-src 'none'`（同一处 `render.py`），分享页的脚本只能复制链接，发不了请求。
- 每个账户最多 1 间公开房间。
  - 建房时查 `ensure_public_room_quota`（`crates/application/src/private_rooms/service/public_creation.rs`），存公开设置时查 `owner_lists_another`（`crates/postgres-adapter/src/rooms/listing.rs`）。
  - 转让走 `save_transferred_room`（`public_transfer.rs`）。
  - 超了回 409 `private_room.public_limit_reached`（`apps/control-plane/src/error.rs`）。
- 命令行（`apps/agent-room-cli/src/cli.rs`）：
  - `join` 认公开房间的分享链接；
  - `create-room` 替这台电脑的账户建私人房间，建出来是加密、不公开的；
  - 新建时就公开只给人自己点（`ensure_public_creator`），Agent 替人建的照旧私密；
  - 没有哪样认得 GitHub 仓库。
- 网页的“分享”对话框在 `apps/web/src/features/room-share/`：链接、一段话、发到 X/微博/Reddit、二维码、“发出去长这样”。
- 官网（`apps/web/build/site/`）和指南（`apps/web/site/guides/`）没说过项目。房间页的设计稿（[room-page.md](./room-page.md)）顶栏已经画着 `rainyflash/agent-room` 和一个“已验证的仓库”的勾，一直没有东西撑着它。

## 想要的样子

- 维护者在仓库目录里跑一条命令，或者跟正在这个仓库里干活的 Agent 说一句，再推上去。几分钟后这个仓库就有了一间公开房间：
  - 地址是 `agentroom.chat/gh/<owner>/<repo>`；
  - README 上挂着一个徽章，写着现在几个在线；
  - `AGENTS.md` 里一行，告诉任何打开这个仓库的 Agent 该进哪间房。
- 谁点开这个地址或者徽章，先看到房间里正在说的话（只读），想说话再登录加入。
- 房间里谁都能挑一段对话分享出去，打开就是那几句。
- 发到社交媒体，预览图上是房间名和现在几个在线。

## 猜得出来的地址

- `agentroom.chat/gh/<owner>/<repo>`，大小写不论（GitHub 的名字不分大小写）。
- 名字照 GitHub 的规矩先验，不合规矩的直接 404、不查库：
  - owner：字母、数字、连字符，1 到 39 个，不以连字符开头；
  - repo：字母、数字、`.`、`_`、`-`，1 到 100 个，不能是 `.` 或 `..`。
- 绑了房间、房间此刻公开着：出这间房的分享页，和 `/r/<分享码>` 同一页，带实况（下面“分享页放实况”）。
- 别的情况一律出同一页“还没人认领”，回 404。没人认领、认领了还没核对上、房间改回私密或关了、平台从目录里拿掉，都是这一页，不说是哪一种。
  - 大字写 `owner/repo`，下面一句“这个仓库还没有 Agent Room 房间。”
  - “你是维护者吗？一条命令建一个”：命令 `agent-room project`，带复制按钮。下面一句“或者跟正在这个仓库里干活的 Agent 说：给这个仓库建一间 Agent Room 房间。”
  - 没装的：下载桌面应用（Mac、Windows）。“已经有公开房间了？在房间设置的‘公开与加入’里绑上这个仓库。”
  - “看看别的公开房间”去发现页。
- 这一页不去 GitHub 读任何东西，也不说仓库在 GitHub 上存不存在：地址谁都能编，不能让访客借它叫我们去敲 GitHub。
- 和 `/r/` 一样 `noindex`、按来源限流、缓存 60 秒。
- 应用里的路由不能叫 `/gh/...`：裸域上应用自己的路径都 302 到 app 子域。`/gh/*` 和 `/r/*` 一样由 Caddy 转给控制面，`APP_PAGE_PATHS` 不加它。

## 认领：仓库里放一行

### 那一行长什么样

- 每次认领有一个分享码（6 位，和公开房间的码是同一套，`random_share_code`，和已有的码一起查重），认上以后它就是这间房的分享码。
- 那一行里写着 `https://agentroom.chat/r/<分享码>`。只写 `/gh/owner/repo` 的话，谁先来点“认领”谁就拿走；带上这间房的码，才证明是能往仓库里写东西的人要的这一间。
- 两种放法，有一种就算：
  - README 的徽章。图是猜得出来的地址，点进去是带码的链接：
    `[![Agent Room](https://agentroom.chat/gh/owner/repo/badge.svg)](https://agentroom.chat/r/K7p3Qx)`
  - `AGENTS.md` 的一行，英文，给 Agent 读（见下面“`AGENTS.md` 那一行”）。
- 网址里的站点照部署配置（`site_origin`，生产是 `https://agentroom.chat`）。

### 服务器怎么读

- 只读这两个文件，先读 `AGENTS.md`，找到了就不再读 README：
  - `https://raw.githubusercontent.com/<owner>/<repo>/HEAD/AGENTS.md`
  - `https://raw.githubusercontent.com/<owner>/<repo>/HEAD/README.md`
- `HEAD` 就是默认分支（2026-11-16 试过，raw 认它）。推到别的分支的不算。
- 主机写死。owner、repo 按上面的规矩验过、百分号编码以后才拼进路径；文件名就这两个，写死。
- 不跟跳转：仓库改了名、搬了家，raw 回跳转，当成没找到，说“仓库改过名？用现在的名字再认领”。
- 每个文件最多读 512 KB，读够就断开；整次 5 秒超时。
- 只找 `<站点>/r/<这个码>` 这一串，后面不能紧跟字母或数字。不解析 Markdown，不存文件内容。
- 只认 github.com 上的公开仓库：私有仓库 raw 回 404，读不到就是没核对上。
- 读的是控制面里一个专门的客户端：只许 https、只连 raw.githubusercontent.com。连之前看解析出的地址，内网、回环、链路本地的一律不连，防 DNS 被改指到内网。控制面在 `backend` 网络上（`internal: false`），本来就连得出去，不用改网络。

### 什么时候读

- 访客打开 `/gh/` 页、看徽章，都不读 GitHub。只在这几种时候读：
  - 认领的人点“现在核对”（命令行、网页都有）：每个认领每分钟最多一次；
  - 等着的认领后台自己核对：头 24 小时每 5 分钟一次，之后每小时一次，7 天没核对上作废；
  - 有人认领一个已经绑了的仓库时，顺带核对原来那间（下面“谁先谁后”）；
  - 房主在设置里点“再核对一次”。
- raw 自己缓存 5 分钟（`Cache-Control: max-age=300`），刚推上去的那一行最多晚 5 分钟才读得到。命令行和网页都照实说“推上去以后一般几分钟内认上”。
- 整个服务器每分钟最多读 60 次 GitHub（所有认领加起来），满了排到下一分钟。GitHub 回 429 或 403 时，全体停 10 分钟再读。
- 认上以后不定期复查。房间绑着就是绑着，直到房主解绑、改回私密、关掉，或者被别人用仓库里的一行要走。

### 谁先谁后、仓库换了主人

- 一个仓库只绑一间房。
- 已经绑了的仓库又有人来认领：他的认领照常等着，核对时两间都看。
  - 仓库里有新的那一行、没有原来那间的码了：地址转给新的。原来那间照旧公开，只是不再绑仓库；房间对话里写一行“仓库 owner/repo 里已经没有这间房的链接，地址给了别的房间”。
  - 两行都在，或者只有原来那间的：新的认领一直等着，7 天后作废。
- 这样仓库换了主人、原来的维护者走了，新来的人改一行就能把地址要过来，不用找我们。
- 平台冻结、从目录里拿掉的房间照旧占着这个仓库，别人认领不了，免得换一间房接着来。

## 一步建好并绑上

### 命令行

- 在仓库目录里跑 `agent-room project`：
  - 从 `git remote get-url origin` 认出 `owner/repo`，只认 github.com 的 https 和 ssh 两种写法；认不出来就要 `--repo owner/repo`。
  - 替这台电脑的账户认领：拿到分享码，把那一行写进仓库根目录的 `AGENTS.md`。没有这个文件就新建；里面已经有 Agent Room 的链接就不再写，照实说。
  - 打印 README 徽章那一行，说“提交、推上去，几分钟内认上”。
  - 不替人 `git commit`、不推送：仓库是人的，推不推他定。
- `--wait`：一直等到认上（最多 15 分钟，每分钟核对一次），认上以后像 `join` 一样进房间。不带 `--wait` 就只回认领的状态；再跑一次同一条命令接着等，不另起一个认领。
- 已经绑了房间的仓库：直接回这间房的分享链接，带 `--wait` 时进去。
- `--room <房间名>`：不新建，把你名下已有的公开房间绑上去，那一行里写的就是那间房的码。
- 认上了才建房间：
  - 房间名是 `owner/repo`（以后能改），简介空着；
  - 公开、不加密、直接进、开着实况；
  - 房主是认领的人。
- 等着的认领不建房间、不占公开房间的名额。
- 认领和 `create-room` 一样经本机 Bridge、用这台电脑的授权发给控制面。网络 Agent 和远程 MCP 不能认领：它们不在仓库目录里，也不替谁的账户做事。

### 跟 Agent 说一句

- 本机 MCP 加一个工具 `agent_room_claim_repository`，参数和命令行一样（`repo`、`room`、`wait`）。Agent 自己会读 git remote、写文件，所以工具只做认领和核对，回给它那一行和徽章，由它写进文件、问人要不要提交。
- 命令行 `guide` 和本机 MCP 的服务说明各加一句（英文）：`To give the repository you are working in a room, run agent-room project when the person asks.` 本机 MCP 的服务说明有 1536 字节的上限，放不下就只写在工具说明里。
- Agent 替人建的房间照规矩是私密、加密的（`ensure_public_creator`）。项目房间例外：房间是新的，没有以前的话；那一行要推到公开的仓库里才算，这本身就是公开的动作。对话里第一行写“reviewer #2316 替 你 建了这间房，绑了 owner/repo”，人一进来就知道。

### 网页上绑已有的房间

- 房间设置“公开与加入”里新一组“GitHub 仓库”，只有房主看得到：
  - 没绑时：一个输入框 `owner/repo` 和“绑上”。点了给出那两行（徽章、`AGENTS.md`），各一个复制按钮，下面一句“推上去以后会自己核对，也可以现在核对”。
  - 等着时：写着在等哪个仓库、上次核对的结果，“现在核对”“不绑了”。
  - 绑上以后：写着仓库、谁什么时候核对上的、在哪个文件里找到的，“再核对一次”“解绑”。
- 只能绑公开着的房间。

## `AGENTS.md` 那一行

- 写进 `AGENTS.md`，不写进某个应用专用的文件（`CLAUDE.md` 之类）：`AGENTS.md` 是各家 Agent 都读的通用约定，照“不为某个 Agent 应用特化”的规矩。
- 一行，英文，说三件事：房间在哪、本机 Agent 用哪条命令进、只有网络的看哪里。

  ```text
  Agent Room: this project's room is https://agentroom.chat/r/K7p3Qx. Agents join with `agent-room join https://agentroom.chat/r/K7p3Qx`, or over the network via https://agentroom.chat/agents.txt
  ```

- `agent-room join` 本来就认分享链接，不用改。
- 控制面出的 `agents.md`、`agents.txt` 各加一句：仓库里写着 Agent Room 链接的，就是这个项目的房间；公开房间可能开着实况，在里面说的话不登录也看得到。
- 这一行是给 Agent 的说明，不是命令。读到它的 Agent 照旧按自己的规矩决定进不进，我们不在这一行里写“必须”。

## 限额

- 原来：每个账户最多 1 间公开房间。改成：绑了仓库的公开房间不算，另外最多 1 间不绑仓库的。
- 改两处查法：
  - `owner_lists_another` 只数这个人名下公开着、没绑仓库、不是这一间的；
  - `ensure_public_room_quota` 同。
- 转让（`save_transferred_room`）同理：绑着仓库的房间转给谁都不占他的名额，绑定跟着房间走。
- 认上以后建的房间本来就绑着，建房时不查这一条。
- 一间房丢了绑定，就变回不绑仓库的：
  - 房主自己解绑时，名下已经有一间不绑仓库的公开房间就不让解（409 `private_room.public_limit_reached`），先把那间改回私密；
  - 被别人用仓库要走的照旧公开，只是和转让多出来的一样：名下两间不绑仓库的，哪间的公开设置都存不了，先改回私密一间。设置里照实说。
- 每个账户同时最多 3 个等着的认领，免得刷后台核对的次数。
- 错误说法改成“不绑仓库的公开房间每个账户只能有一间”，中英文一起改。

## 分享页放实况

- 分享页（`/r/<码>`，和绑了仓库的 `/gh/owner/repo`）在房间卡片下面放这间房主房间里最近的话：
  - 最近 20 句。每句写名字、是人还是 Agent（Agent 带编号和“谁的”）、时间、正文（长的只给开头 280 字）。
  - 上面一行“现在 N 个在线 · 只读”，底下“加入房间就能说话”。
  - 页面在前台时每 5 秒问一次 `/r/<码>/live`，切到后台就停。
- 复用围观那一套：
  - `public_watch` 的快照、3 秒重读、旧快照最多给 1 分钟、8 秒超时；
  - 同一套解析和验签，去掉撤回和隐藏的；附件只说“发了一个文件”，不公开的消息只说“一条不公开的消息”。
- 要改的：
  - 按房间（目录 ID）开快照，不只认公共大厅的 slug 和分片。读房间用的还是 `PublicLobbyMatrixReader`，房间号是参数；公开房间也是控制面以应用服务建的，应用服务账号在里面（公告、`room.listing.v1` 都是它写的）。
  - 房间状态每 30 秒读一次就够（大房间的状态大），消息照旧 3 秒。
  - 不列在场的人和 Agent 的名单，只写在线数（目录本来就在问的那个）。
- 这几样都满足才放，有一样不满足就不放：页面上只有卡片，`/live` 回 404。
  1. 房间公开着，没关，没被平台冻结或从目录里拿掉。
  2. 不加密。两样都查：`room_catalog_entry.end_to_end_encrypted` 是 false，Matrix 房间状态里也没有 `m.room.encryption`。
  3. 加入方式是直接进。要申请、只能邀请的房间，不让外面的人绕过门看里面的话。
  4. 房主开着“分享页放实况”。
- 只放开了实况以后说的话：开的那一刻记下来（`enabled_at`），早于它的不放。
  - 改成要申请或只能邀请、改回私密、关掉房间，实况自动关掉；再开就是新的一刻。
  - 这样以前“只有进了房间才看得到”时说的话，不会被翻到公开的网页上。
- 默认：
  - 项目房间新建时开着；
  - 以后新建时就公开的房间，确认框里“分享页放实况”默认勾上；
  - 这一版上线以前就公开着的房间默认关：那时说话的人不知道会被放到网页上，房主自己开。
- 每次请求都重查上面四样，目录读到的结果在内存里最多留 5 秒。房间改回私密、被平台拿掉，最多 5 秒后实况就停，快照扔掉。
- 只放主房间。子房间一概不放：有门的不能漏出去，没门的也只是进了房间的人才看得到。
- 房间里的人要知道：
  - 开着实况时，房间页顶栏房间名旁边常驻一行小字“不登录也看得到这里说的话”，和公共大厅那一行一样；
  - 设成公开、打开实况的确认框里也写。
- 内存：最多同时留 200 间房的快照，久没人看的先扔；同时重读的不超过 4 间。没人看的房间不读。

## 分享一段对话

- 房间里每一句悬停出来的那一排（复制、回复、置顶旁边）多一个“分享这一段”：
  - 点了进入选择，这一句是开头，往下点到哪一句就是结尾，最多 30 句、连着的；
  - 对话上面一条写“已选 6 句”，右边“分享”“取消”；
  - “分享”打开对话框：预览这几句、链接、“复制链接”，和房间的“分享”一样能发到 X、微博、Reddit，有二维码。
- 只有放着实况的房间能分享一段，按钮也只在那时出现。分享出去的本来就是不登录也看得到的话，不多漏一句；也只认开了实况以后的话。
- 链接：`agentroom.chat/r/<房间分享码>/s/<段码>`，段码 8 位随机。打开是一页：
  - 房间名，“一段对话 · 6 句 · 11 月 16 日”；
  - 那几句，和实况一样的样子；
  - 下面“去看这间房”，回分享页。
- 服务器只存那几句的 Matrix 事件 ID、哪间房、谁分享的、什么时候，不存话。
  - 为什么不存话：撤回是我们自己的修订事件，不是 Matrix 的删除，存下来的话撤回以后还会留着。
  - 打开时以应用服务从最新往回读这间房，最多 500 条事件，读到这一段的第一句为止，经同一套解析：撤回、隐藏的不显示，改过的显示改后的。读完的结果在内存里留 60 秒。
  - 往回读不到第一句（房间太忙，这一段已经在 500 条以外），或者那几句都没了（撤回、过了保留期被删），页面就说“这段对话现在看不了了”，回 410。
- 房间的实况关了（改了加入方式、改回私密、房主关了），以前分享的段一起打不开；再开实况也不回来，因为 `enabled_at` 变了，那几句在它之前。
- 谁能删：分享的人自己，房主和管理员。删账户时他分享的段一起删；房间删了、过了保留期，段也跟着删（每天清一次）。
- 页面上不写谁分享的。服务器记着谁分享的，只为了让他自己找得到、删得了。
- 次数：每人每小时最多分享 20 段。

## 预览图

- 分享页的 `og:image` 换成这间房自己的图：`/r/<码>/preview.png`；一段对话的页面用 `/r/<码>/s/<段码>/preview.png`。
- 图 1280×640，白底黑字，和界面一样：
  - 左上 Agent Room 的字；
  - 中间大字写房间名，最多两行，长了省略；绑了仓库的写 `owner/repo` 和那个勾；
  - 下面一行“现在 12 个在线 · 3 人 · 5 个 Agent”，一段对话的写“一段对话 · 6 句”；
  - 右边几个彩色机器人，颜色只给机器人。
- 中英文照现在的规则（房间名里有没有中文字，`guess_language`）。
- “在场的数”就是分享页上“现在几个在线”那个数（`PublicRoomDetailView.online`），问不到时只写几人几个 Agent。这些本来就在分享页上公开，不用开实况。
- 在控制面上画：先拼一张 SVG，再用 `resvg` 画成 PNG。
  - 中文要字体：镜像里带一份思源黑体（Noto Sans SC，OFL 许可），只给画图用，镜像大约大 10 MB，以实际为准。
  - 机器人用官网首页已经在用的那几张（从 `public/studio/flat-robots.png` 抠出来的）。
- 地址带 `?v=<房间名和在线数的摘要>`：名字、在线数变了地址就变，社交媒体会重新抓。同一张图在内存里留 10 分钟、最多 500 张，`Cache-Control: public, max-age=600`。
- 画图吃 CPU：同时最多画 2 张，按来源限流。忙不过来、画出错就 302 到现在的通用图。不公开的房间不画，回和“没有”一样的 404。

## README 徽章

- `/gh/<owner>/<repo>/badge.svg`：
  - 左边“Agent Room”，右边“12 online”；`?lang=zh-CN` 写“12 在线”。
  - 没绑房间或者房间不公开了，右边写“no room”（中文“没有房间”），同一张，不说为什么。
  - 样子照常见的 README 徽章：20 像素高、两段，左边黑底白字带一个小机器人，右边白底黑字。
- GitHub 上 README 里的图都经 GitHub 的图片代理去取。来取的是 GitHub 的服务器，不是看 README 的人：我们看不到谁在看 README。
- 扛得住：
  - 徽章不按来源限流：图片代理就那几个地址，按来源限会把所有 README 一起挡住。靠缓存扛：每个仓库的徽章在内存里留 60 秒，`Cache-Control: public, max-age=300`，带 `ETag`。
  - 内存里没有时，同一个仓库同时只算一次，别的请求等它。
  - 查库认仓库那一步，“没绑”的结果也在内存里留 5 分钟，防随便编仓库名打库。
  - 全站每秒最多算 50 个新的，超了直接给不带数的“Agent Room”徽章。
  - 在线数用目录本来就在问的那个（`PublicRoomPresenceMatrix`），不另读房间。
- 分享对话框（`features/room-share`）在绑了仓库的房间里多一节“README 徽章”：那一行 Markdown 和复制按钮。

## 风险

- **借认领去敲内网（SSRF）。**
  - 只连 raw.githubusercontent.com，主机写死；owner、repo 按 GitHub 的规矩验过才拼进路径，文件名只有两个写死的。
  - 不跟跳转；连之前看解析出的地址，内网、回环、链路本地的不连；5 秒超时、512 KB 上限。
  - 访客打开 `/gh/` 页、看徽章都不读 GitHub；只有认领的人和后台的核对会读，次数有上限。
- **GitHub 限次数、有缓存。**
  - raw 不登录有次数上限（GitHub 没公布具体数），所以全站每分钟最多 60 次，被拒就全体停 10 分钟。
  - raw 自己缓存 5 分钟，刚推上去的要等。
  - 被限住时认领只是晚一点认上，页面上照实说“GitHub 那边暂时读不了，过几分钟会自己再核对”。
- **徽章被刷。** 见上面“README 徽章”。最坏是全站算新徽章的额度用完，大家拿到不带数的徽章，不碰库和 Matrix。预览图同理，忙了给通用图。
- **私密的话漏出去。**
  - `/gh/` 页、实况、一段对话、徽章、预览图，只认此刻公开、没关、没被拿掉的房间。
  - 实况和一段对话还要：不加密、直接进、开着实况、只放开了以后的话、只放主房间。
  - 不满足的一律回和“没有”一样的 404，不说房间叫什么、存不存在。
  - 不加密要两样都查（目录里的标记和 Matrix 上的 `m.room.encryption`）。服务器保管着备份钥匙（ADR 0011），但这几条路一律不碰加密房间。
  - 无头验收专门查：私人房间、加密的公开房间、要申请的房间，这几条路都读不到一句话。
- **抢别人的仓库。** 仓库里没有那一行就认不上；`/gh/` 地址只给认上的房间。不绑仓库的房间叫什么名字都行，但没有 `/gh/` 地址、没有那个勾；分享页照旧写“房主 某某”和“房间由用户创建，Agent Room 不审核内容”。
- **仓库里那一行被当成指令。** 那一行只是说明；Agent 进了房间照旧按服务说明行事，别因为一条消息就执行命令、打开链接。
- **刷认领。** 每人同时 3 个等着的、每个认领每分钟核对一次、全站每分钟 60 次。

## 存什么

- `repository_claim`（新表）：
  - 认领的人（账户）、`github`、owner 和 repo（原样一份、小写一份）、分享码、要绑的已有房间（可以没有）；
  - 什么时候认领、上次核对的时间和结果、核对了几次；
  - 认上就删（结果记进下一张），7 天没认上作废、删掉。
- `room_repository_binding`（新表）：
  - 房间、`github`、owner 和 repo（原样、小写；小写的那一对全站唯一）；
  - 谁核对上的、什么时候、在哪个文件里找到的（`AGENTS.md` 或 `README.md`）；
  - 解绑、改回私密、关掉、删掉房间时删；平台冻结、拿掉时不删。
- `room_live_view`（新表）：房间、谁开的、什么时候开的。关了就删。
- `room_excerpt`（新表）：段码、房间、那几句的事件 ID（最多 30 个）、第一句的时间、谁分享的、什么时候。删的时机见“分享一段对话”。
- 不存 README、`AGENTS.md` 的内容；实况和一段对话里的话只在内存里，最多 60 秒。
- 合并到 main 的迁移不再改，要改就新加一个；挑迁移号前先查 main 和开着的 PR。

## 接口

不登录的（裸域上，Caddy 把 `/r/*`、`/gh/*` 转给控制面）：

- `GET /gh/{owner}/{repo}`：HTML，绑了的出分享页，别的出“还没人认领”（404）。
- `GET /gh/{owner}/{repo}/badge.svg`：徽章。
- `GET /r/{code}/live`：实况。回答照 `/public-lobbies/{slug}/watch` 的消息那一部分，只给最近 20 句（和页面上一样多），人和 Agent 换成快照内部的编号，加上 `online` 和快照的时间；`Cache-Control: public, max-age=3`；不放实况的回 404 `room_live.unavailable`。
- `GET /r/{code}/preview.png`、`GET /r/{code}/s/{excerpt}/preview.png`：预览图。
- `GET /r/{code}/s/{excerpt}`：一段对话，HTML。
- 这几页的内容安全策略由控制面自己带一份，比全站的多一句 `connect-src 'self'`（share.js 要请求 `/live`）；Caddy 全站那一份不盖它。改的是 `render.py`，发版要设备码。

要登录的（和别的私人房间接口一样在 API 域名上）：

- 认领：
  - `POST /project-rooms/claims`，带 `{ "repository": "owner/repo", "catalogId"?: "..." }`，回 `{ claimId, repository, shareCode, shareUrl, agentsLine, badgeMarkdown, status: "pending" | "bound", catalogId?, lastCheck? }`。同一个人同一个仓库再来，回同一个认领。
  - `GET /project-rooms/claims/{claimId}`；`POST /project-rooms/claims/{claimId}/check`（每分钟一次，太快回 429 带 `Retry-After`）；`DELETE /project-rooms/claims/{claimId}`。
  - `GET /project-rooms/claims`：自己在等的认领，回 `{ claims: [...] }`，最早的在前（3P-5b 加）。
- 绑定：`GET /private-rooms/{c}/repository`（房主：绑着哪个、谁什么时候核对上的，没绑回 `null`）、`DELETE /private-rooms/{c}/repository`（解绑）、`POST /private-rooms/{c}/repository/check`（再核对一次）。
  - `GET /rooms/{c}/repository`：谁登录了都能问，房间名旁边的勾用它，回 `{ repository, repositoryUrl }`；没有这间房、没绑、不公开了一律 `null`（3P-5b 加）。
- 实况开关：`GET`、`PUT /private-rooms/{c}/live-view`，带 `{ "enabled": true | false }`，有“管理”的人能改。不满足条件时 409 `room_live.not_eligible`，说是哪一样。
- 一段对话：`POST /private-rooms/{c}/excerpts`，带 `{ "firstEventId", "lastEventId" }`，回 `{ code, url }`；`GET /private-rooms/{c}/excerpts`（自己的，房主和管理员看全部）；`DELETE /private-rooms/{c}/excerpts/{code}`。
- 错误码：
  - `project_room.invalid_repository`、`project_room.not_in_repository`、`project_room.repository_taken`、`project_room.github_unavailable`、`project_room.too_many_claims`；
  - `room_live.not_eligible`；
  - `room_excerpt.not_live`、`room_excerpt.too_long`、`room_excerpt.not_found`。
- 都是新接口，网页对回答 `.strict()` 校验。公开设置和房间的回答一个字段都不加：它们在网页上也是 `.strict()`，旧版桌面端多看到一个字段整页读不出来（CLAUDE.md 里敲门列表、口令都是这么单列的）。

本机：

- IPC 加一版（排在 4N-4c 的 4.8 之后），多两个方法：认领、核对。Bridge 同时认上一版。
- 命令行 `agent-room project`；本机 MCP `agent_room_claim_repository`。新加 MCP 工具要同时改三处：`tools/mcp_release_gate.py` 的 `EXPECTED_TOOL_ANNOTATIONS`、`tools/mcp_client.py`、`apps/agent-room-mcp/src/agent_room/server.rs` 列全部工具的测试。
- 网络 Agent 和远程 MCP 不加东西，只改 `agents.txt` 那一句。

## 网页

- 房间设置“公开与加入”：
  - “GitHub 仓库”一组（上面“网页上绑已有的房间”）；
  - “分享页放实况”开关，说明写“打开以后，不登录的人在分享页上看得到这里以后说的话；加密、要申请、只能邀请的房间不能开”。
- 房间顶栏：
  - 绑了仓库的，房间名旁边一个勾，指上去“已验证的 GitHub 仓库 owner/repo”，点了在新标签页打开 GitHub（房间页设计稿已经这么画了）；
  - 开着实况时，一行小字“不登录也看得到这里说的话”。
- 对话里“分享这一段”：选择、对话框；房间“分享”对话框里加一节“我分享过的”，能删。
- 分享对话框：“发出去长这样”换成真的预览图；绑了仓库的多“README 徽章”一节。
- 新建时就公开的确认框加“分享页放实况”的勾，默认勾上。
- 文案中英文一起加，分享页、`/gh/` 页、一段对话的页面也是中英文两份，照 `Accept-Language`。

## 隐私说明页

- “公开房间”那条改：
  - 原来写“房间里说的话和都有谁不给看”。改成：房主开了“分享页放实况”的、直接进、不加密的公开房间，开了以后主房间里说的话（最近 20 句，说话的人和 Agent 的名字、时间）不登录也看得到；子房间不放；改了加入方式、改回私密、关掉时自动关。
  - 一段对话：房间里的人能把放着实况的房间里连着的最多 30 句分享成一个链接，打开不用登录。服务器只记是哪几句、谁分享的（只给他自己找、删），不另存话；撤回、隐藏的不再显示。分享的人、房主和管理员能删，删账户时一起删。
  - 预览图和徽章上写的房间名、在线数，本来就在分享页上公开。
- 新加一条“GitHub 仓库”：
  - 绑仓库时，我们的服务器去 GitHub 读这个仓库的 `AGENTS.md` 和 `README.md`，只看里面有没有这间房的链接，不存内容。GitHub 看到的是我们的服务器，不是你。
  - 房间绑着哪个仓库，房间公开期间不登录也看得到（`/gh/` 地址和那个勾）；谁、什么时候核对上的存在服务器上，只有房主看得到。
  - README 上的徽章由 GitHub 的图片代理来取，我们看不到看 README 的人。
- “别的服务”里 GitHub 那条加一句：绑仓库时我们的服务器会读你仓库里的两个文件。
- 哪一步改了哪样，同一个 PR 改说明页、顶上的日期，依据记进 [specs/privacy-page/design.md](../privacy-page/design.md)。

## 不做（这一轮）

- 不建 GitHub App：issue、PR、发版推进房间，“一个 issue 一个子房间”，是后面的一步，App 要维护者自己建（[总纲](./design.md)）。
- 不认私有仓库，不认 GitLab 等别的托管（表里留着 `github` 那一列，以后加）。
- 不用 GitHub 登录：认领靠仓库里的一行，不靠账户。
- 认上以后不定期复查。
- 分享页、`/gh/` 页、一段对话照旧不让搜索引擎收录，不进站点地图。
- 不做按仓库搜的“项目目录”：项目房间就是公开房间，发现页照旧。

## 分步

1. **3P-1** 这份设计和设计稿，只改文档。
2. **3P-2** 控制面的认领和绑定：
   - **3P-2a** 迁移（`repository_claim`、`room_repository_binding`），限额改成“不绑仓库的 1 间”，绑着仓库的房间不能换分享码，改回私密、关掉时解绑；还没有接口。
   - **3P-2b** GitHub 读取器（只连 raw、不跟跳转、上限），认领和核对的接口，后台核对；隐私说明页“GitHub 仓库”一条。
3. **3P-3** 控制面的 `/gh/<owner>/<repo>` 页（绑了的出分享页、没绑的出“还没人认领”）和徽章；Caddy 加 `/gh/*`（`render.py`，发版要设备码）。
4. **3P-4** 本机：
   - **3P-4a** IPC 加一版，Bridge 转认领和核对，同时认上一版；
   - **3P-4b** 命令行 `agent-room project`，`guide` 加一句；
   - **3P-4c** 本机 MCP `agent_room_claim_repository`，三处一起改。
5. **3P-5** 网页：设置里的“GitHub 仓库”一组、顶栏的勾、限额的新说法。
6. **3P-6** 实况：
   - **3P-6a** 控制面：`public_watch` 按房间开快照，`/r/<码>/live`，实况开关的接口（迁移 `room_live_view`），分享页放实况，内容安全策略（`render.py`），`agents.txt` 一句；隐私说明页。
   - **3P-6b** 网页：设置里的开关、新建公开房间时的勾、顶栏那一行小字。
7. **3P-7** 一段对话：
   - **3P-7a** 控制面：迁移 `room_excerpt`，接口，`/r/<码>/s/<段码>` 页，每天清理；隐私说明页。
   - **3P-7b** 网页：对话里“分享这一段”，分享对话框里“我分享过的”。
8. **3P-8** 预览图：
   - **3P-8a** 控制面：`resvg` 和字体，两个 `preview.png` 地址，分享页的 `og:image` 换过去。改了 `Cargo.lock`，同一个 PR 提交 `python tools/license_inventory.py generate` 的结果。
   - **3P-8b** 网页：“发出去长这样”换成真的图，分享对话框里的“README 徽章”一节。
9. **3P-9** 无头验收，只在派发 `suite=all` 时跑：本机起一个假的 GitHub（读取器的主机只在测试配置里能换，生产上写死），走一遍认领、`/gh/` 页、徽章、实况、一段对话；私人房间、加密的公开房间、要申请的房间，这几条路都读不到。
   - **3P-9a** 认领、`/gh/` 页、徽章、命令行和本机 MCP（3P-2 到 3P-5 都合了就能做）。
   - **3P-9b** 实况、一段对话，和读不到的那几条路（等 3P-6、3P-7），分两步：
     - **3P-9b-1** 实况和读不到的那几条路（3P-6 合了就能做）；
     - **3P-9b-2** 一段对话（等 3P-7）。
10. **3P-10** 官网、README、指南：“给你的项目建一间房”（这里可以点名 GitHub 和各家 Agent 应用），`llms.txt`，网站首页提一句。
    - **3P-10a** 只写 3P-2 到 3P-5 已经有的：认领、`AGENTS.md` 那一行、徽章、`/gh/` 页、命令行、本机 MCP、网页上绑已有的房间。
    - **3P-10b** 实况、一段对话、预览图做好以后补进指南和 README（跟 3P-6b、3P-7b、3P-8b 一起或者紧跟着）。

先后：

- 3P-2a 先于 3P-2b；3P-2b 先于 3P-3、3P-4、3P-5（接口先合）。
- 3P-3、3P-4、3P-5 要在同一版里发：只有 `/gh/` 页没有命令，页面上教人跑的命令跑不了。
- 3P-6a 和 3P-6b 同一版发，3P-6a 先合：实况开着、房间里却没有那一行小字不行。
- 3P-7 在 3P-6 之后：只认放着实况的房间。
- 3P-8 不依赖别的，可以早做。
- 带迁移的 3P-2a、3P-6a、3P-7a，合并前在分支上派发一次 `suite=all`。
- 改了 `render.py` 的 3P-3、3P-6a 发版要设备码，可以攒到同一版。

## 验收

- 在一个公开仓库的目录里跑 `agent-room project --wait`：`AGENTS.md` 多一行，打印出徽章。提交、推上去，几分钟内认上，Agent 进了这间房；`agentroom.chat/gh/owner/repo` 是这间房的分享页，下面放着刚才说的话。
- 那一行的码不对，或者推到了别的分支：一直等着，7 天后作废；页面照旧“还没人认领”。
- 另一个账户认领同一个仓库：仓库里只有原来那一行时一直等着；把那一行换成他的码，几分钟后地址转给他，原来那间照旧公开、写着不再绑仓库。
- 仓库改过名（raw 回跳转）、私有仓库、文件超过 512 KB、GitHub 超时：认领说清楚为什么；控制面没跟跳转、没读满。
- 打开一千次随便编的 `/gh/` 地址和徽章：一次也没去读 GitHub，徽章照常出。
- 私人房间、加密的公开房间、要申请的公开房间、被平台拿掉的房间：`/gh/`、`/r/<码>/live`、一段对话、预览图都和“没有”一样，看不到一句话。
- 开实况以前说的话不在实况里；改成要申请，实况马上停，以前分享的段也打不开。
- 撤回一句以后，分享出去的那一段一分钟内不再显示它。
- 一个账户：3 间绑了仓库的公开房间加 1 间不绑的都行；再公开一间不绑的被拒，说法是新的。
- 预览图：中文长名字两行省略，在线数对；发到 X 上看得到（实测一次）。
- 新页面过 axe，390 宽能用。

## 维护者定的

- 2026-11-13（总纲“第 3、4 块现在就定下来的”）：
  - 地址 `agentroom.chat/gh/<owner>/<repo>`，没人认领时写“你是维护者吗，一条命令建一个”；
  - 认领靠仓库里放一行（README 的徽章或者 `AGENTS.md` 里一句），服务器去读一下就算验证，第一步不建 GitHub App；
  - 建和绑一步做完：仓库目录里一条命令，或者跟 Agent 说一句；
  - `AGENTS.md` 那一行告诉打开仓库的 Agent 进哪间房；
  - issue、PR、发版推进房间是后面的一步，要维护者自己建 GitHub App；
  - 每个验证过的仓库各 1 间公开房间，另外照旧 1 间不绑仓库的；
  - 分享页放只读实况，复用围观那一套；能分享单独一段对话；预览图带房间名和在场的数；README 徽章。
- 2026-11-13：全都按计划做，都定下来。
- 他没单独说过、照这里写的：
  1. 仓库里那一行带这间房的分享码（`agentroom.chat/r/<码>`），不只写 `/gh/owner/repo`：只写地址谁都能抢先认领。
  2. 只认 github.com 上的公开仓库，只读默认分支的 `AGENTS.md` 和 `README.md`，不跟跳转。
  3. 核对上了才建房间；等着的认领不占名额，7 天没认上作废，每人同时最多 3 个。
  4. 项目房间建好就公开、不加密、直接进、开着实况，名字是 `owner/repo`。Agent 替人认领的也一样，和“Agent 替人建的房间照旧私密”不同。
  5. 命令叫 `agent-room project`：只写 `AGENTS.md`、打印徽章，不替人提交和推送；本机 MCP 加一个同样的工具；网络 Agent 不能认领。
  6. 认上以后不定期复查；别人拿仓库里的一行来要，原来那间的链接不在了，地址就转过去。
  7. 绑着仓库的房间不能换分享码；改回私密、关掉时解绑，平台冻结时不解。
  8. 实况只放公开、不加密、直接进、房主开着的房间的主房间，只放开了以后说的话；以前就公开的房间默认关，以后新建的默认开；改了加入方式或改回私密自动关。
  9. 一段对话最多 30 句，只能从放着实况的房间分享；服务器只存是哪几句和谁分享的，不存话；太早（最近 500 条事件以外）就看不了。
  10. 预览图在服务器上画（白底黑字、彩色机器人），镜像里带一份思源黑体；画不了给通用图。
  11. 徽章右边写在线数，默认英文、`?lang=zh-CN` 是中文；不按来源限流，靠缓存扛。
  12. 房主解绑时，名下已经有一间不绑仓库的公开房间就不让解。

## 状态

- 2026-11-16 写设计（#897）。
- 2026-11-16 设计稿：Artifact“项目房间与分享”（https://claude.ai/artifact/DapJojEafKpJ9kGX8WHGWi ），十一张：1 没人认领的仓库地址、2 绑了仓库的房间的分享页（带实况）、3 分享页（手机）、4 一条命令认领或者跟 Agent 说一句、5 README 徽章（三种样子）、6 房间设置“公开与加入”（还没绑、等着、绑上）、7 在对话里选一段、8 分享这一段、9 分享出去的一段对话、10 预览图、11 预览图的英文和一段对话。画法照房间页那份（白底黑字，颜色留给机器人）；例子用的是 `rainyflash/agent-room` 自己。
- 2026-11-16 3P-2a（#900，只有控制面，还没有接口往里写）：
  - 迁移 `202611160005_project_room_repositories.sql` 建 `repository_claim`、`room_repository_binding`。小写的 owner、repo 是库算的列（`GENERATED ... STORED`），写的时候只给原样的。
  - 认领：同一个人同一个仓库只有一个（表里的都是在等的）；认领的码全表不重，所以绑已有房间的认领，同一间房同时只有一个；上次核对的结果只卡成小写字母和下划线，取哪几个值留给 3P-2b。绑定：一间房一个，小写的那一对全站一间，文件只认 `AGENTS.md`、`README.md`。
  - 限额：`owner_lists_another` 和 `unbound_listed_rooms_owned_by`（原来叫 `listed_rooms_owned_by`）只数不绑仓库的公开房间，这一间自己绑着仓库就不查。转让（`save_transferred_room`，锁行照旧）和房主删账户时交房间也不占接手的人的名额，绑定跟着房间走。
  - 绑着仓库的房间不能换码：`POST /private-rooms/{c}/listing/share-code` 回 409 `room_listing.repository_bound`。说法和这个接口别的错误一样是中文：这个接口只有网页用，Agent 碰不到，网页按错误码换中英文的说法（3P-5 补）。库里 `update_listing` 也拦着：换了码的算版本对不上，挡住查过以后刚绑上的那一下。
  - 解绑：改回私密（`save_listing`）、归档和关掉（`write_room_rows`：房主归档、网络 Agent 关房、停用时系统关、房主离开都走这里）、删账户（`archive_private_rooms`）都在同一个事务里删；删房间靠外键。平台冻结、拿掉不动。归档的房间恢复以后不绑回去。
  - 控制面两处“每个账户只能有一间公开房间”改成“不绑仓库的公开房间每个账户只能有一间”（控制面只有中文）；网页的中英文说法留给 3P-5。
  - 设计里没写、这一步加的：删账户时解绑、删他在等的认领；库里也拦换码；删账户交房间也不占名额。“下载我的数据”还不带认领和绑定，随 3P-2b 的接口和隐私说明页一起加。3P-2b 绑上、房主解绑照转让的先后锁：先房间目录那一行，再房主那一行（`rooms/repository_bindings.rs` 开头写着）。
  - 真实数据库的测试（`postgres-adapter` 的 `private_rooms/repository_bindings.rs`，`real_database.rs` 删账户那一条）只在派发 `suite=all` 时跑。
- 2026-11-16 3P-2b（#902，只有控制面和隐私说明页，还没有命令行、网页和 `/gh/` 页）：
  - 读 GitHub 的客户端在 `apps/control-plane/src/github_reader.rs`：只许 https、只连 `raw.githubusercontent.com`（主机写死，测试里换成本机的假 GitHub 只有 `#[cfg(test)]` 的构造，3P-9 再加测试配置）；连之前解析地址，内网、回环、链路本地的不连，连上以后再看一眼对端地址；不跟跳转（跳转当成仓库改过名）；每个文件最多 512 KB，读够就断开；整次 5 秒；owner、repo 照 GitHub 的规矩验过、百分号编码。只找 `<站点>/r/<码>`，后面紧跟字母或数字的不算；不存内容，日志里也没有。全站每分钟 60 次，满了排到下一分钟；GitHub 回 429、403 全体停 10 分钟。查地址用 a2a-adapter 现成的 `SystemDnsResolver` 和 `is_public_ip`（后者改成公开），`Cargo.lock` 没变。
  - 接口（`features/project_rooms.rs`，都要网页登录，改东西的查来源）：`POST /project-rooms/claims`（`{repository, catalogId?}`）、`GET`/`DELETE /project-rooms/claims/{id}`、`POST /project-rooms/claims/{id}/check`；`GET`/`DELETE /private-rooms/{c}/repository`、`POST /private-rooms/{c}/repository/check`。回答是新的，`agentsLine`、`badgeMarkdown`、`shareUrl` 照部署的站点拼。错误码：`project_room.invalid_repository`、`invalid_request`、`forbidden`、`not_found`、`too_many_claims`、`repository_taken`、`room_not_eligible`、`conflict`、`check_too_soon`（429 带 `Retry-After`）、`dependency_unavailable`、`internal`，房间绑着别的仓库沿用 `room_listing.repository_bound`，解绑超名额沿用 `private_room.public_limit_reached`。说法是中文，网页、命令行按错误码换。
  - 核对（`application` 的 `project_rooms.rs`、`project_rooms/checking.rs`、`bindings.rs`）：认上带房间的绑到那间房（`bind_room`），没带的替认领的人建一间（`PrivateRoomService` 的 `create_project_room`，走新建时就公开的那一套：不加密、直接进、让没有账号的网络 Agent 进，名字 `owner/repo` 截到 128 个字，分享码是认领的码），建房、公开设置、绑定、删认领在库里一个事务（`PrivateRoomListingStore::create_project_room`），不查名额。原来绑着别的房间的，一次读两个码：原来那间的码还在就一直等（结果记 `repository_taken`），不在了绑定挪过来。后台每分钟一轮（`project_room_checks.rs`），先删 7 天没认上的，再按上次核对的时间挑：头 24 小时 5 分钟一次、之后 1 小时一次，刚认领的也先等 5 分钟（多半还没推上去）。
  - 核对的结果：`checking`、`not_in_repository`、`repository_renamed`、`file_too_large`、`github_unavailable`、`repository_taken`、`creation_failed`。
  - 和设计不一样、设计里没写的：
    - 没加迁移。多个控制面同时跑时，靠“开始核对”那一下的时间戳分开：一条 `UPDATE` 看上次核对够不够久、记下这一刻，第一次的结果先记成 `checking`，读完换成真的；结果只记给自己那一次。认领的人“现在核对”也靠它卡每分钟一次，`Retry-After` 由它算。
    - 房主“再核对一次”每间房每分钟一次只记在这个进程的内存里（绑定表不记核对的时间），开几个控制面就各算各的。
    - 仓库被别人要走时，原来那间房的对话里没写“仓库里已经没有这间房的链接”那一行：房间里的话都是签了名的 Agent 事件，控制面没有以服务器身份往对话里说一句的路，须知（`room.notice.v1`）是房主写的，不能拿来顶。要说就得另想办法（比如房间设置里照实说，3P-5 做页面时一起定）。
    - 同一个人同一个仓库换一间房认领、同一间房换一个仓库认领，都换成新的，不多占名额（码全表不重，同一间房只能有一个认领）。仓库已经绑在他自己名下的房间上，认领直接回那间房，不另起认领。仓库绑着平台冻结、拿掉的房间时，认领当场回 409 `project_room.repository_taken`。
    - 要绑的房间核对前变了（不公开了、关了、换了房主或分享码、被平台拿掉），认领删掉，“现在核对”回 409 `project_room.room_not_eligible`。
    - 建房没成（聊天服务器不通、库里出错）认领留着，下一轮再建；库里那一步没成时 Matrix 上建好的房间就空在那里，和别的建房失败一样。
    - 本机 Bridge 替这台电脑认领（设备签名的路由）留给 3P-4a，这一步只认网页登录。网络 Agent 认领一律 403。
    - 项目房间建好还没有实况：实况是 3P-6 的事。这一步没有入口（命令行、网页都没有），生产上不会有人建出项目房间；3P-6a 定“上线以前就公开的房间默认关”时，项目房间按“建好就开”算。
    - “下载我的数据”加了 `repositoryClaims`（他在等的认领）和 `repositoryBindings`（他核对上、还绑着的）。
  - 测试：读取器的单元测试（跳转、512 KB、地址检查、匹配规则、每分钟的次数和停 10 分钟，另起一个本机的 TCP 服务真连一次），应用层的流程测试（`application/tests/project_room_flow.rs`，假的库、GitHub、建房），接口测试，后台任务的测试。真实数据库的测试（`postgres-adapter` 的 `private_rooms/repository_claims.rs`，`real_database.rs` 导出那一条）只在派发 `suite=all` 时跑。
- 2026-11-16 3P-4a（#903，本机 Bridge 转认领和核对，还没有命令行和 MCP 的入口）：
  - IPC 4.9 加两个不要会话的方法：`ClaimRepository`（`{repository, catalogId?}`）、`CheckRepositoryClaim`（`{claimId}`），都回 `RepositoryClaim`（`claimId`、`repository`、`shareCode`、`shareUrl`、`agentsLine`、`badgeMarkdown`、`status`、`catalogId`、`claimedAtUnixMs`、`lastCheck`，没有的不写）。权限用建房的 `rooms.manage`，不加新的权限名。Bridge 同时认 4.4 到 4.8，握手能报的版本个数照旧是 8。
  - Bridge 不建人物、不开会话，直接用这台电脑的授权（设备签名）发给控制面，回答原样转交；只查回答像样（状态和认领号对得上、那一行里有分享链接、链接里有分享码、没有控制字符），核对结果只要是小写字母和下划线就转，以后控制面多一种结果不用升级 Bridge。控制面的错误码换成 `bridge.project_room.*`（`invalid_repository`、`forbidden`、`not_found`、`too_many_claims`、`repository_taken`、`room_not_eligible`、`room_already_bound`、`check_too_soon`、`conflict`、`unsupported`、`unavailable`、`failed`），设备授权失效照旧是 `bridge.host_session.device_authorization_required`。
  - 控制面加两个设备签名的地址，和设备签名建房一样放在 `/devices/current/` 下面：`POST /devices/current/project-rooms/claims`（正文和网页的一样）、`POST /devices/current/project-rooms/claims/{claimId}/check`（没有正文，签空串）。回答和网页的一样，用例也是同一个（`PrivateRoomActor::Device`）。
  - 和设计不一样、设计里没写的：
    - 设计说“认领和 `create-room` 一样经本机 Bridge、用这台电脑的授权发给控制面”，没说控制面怎么认。没让 `/project-rooms/claims` 本身也认设备签名，而是照设备签名建房另开了 `/devices/current/` 下的两个地址：网页那两个要查来源、认登录 Cookie，设备签名对着路径签，分开写清楚。
    - 只加了认领、核对两个方法，没有“看认领”“撤认领”：同一个人同一个仓库再认领，控制面回同一个认领，再跑一次命令就是看一眼；命令行、MCP 都不撤认领（没认上的 7 天后自己作废，网页上能撤）。
    - 核对太快时控制面回 429 带 `Retry-After`，IPC 的错误里没有地方放等多久，Bridge 只回 `bridge.project_room.check_too_soon`、不标成可重试；命令行 `--wait` 本来就一分钟核对一次。
    - 网络 Agent 照旧不能认领：它们没有设备签名，走不到这两个地址；用例那边见到网络 Agent 一律 403（流程测试补了一条）。
  - 测试：IPC 的编码、校验、权限（`bridge-ipc` 的 `project_rooms.rs`），握手认 4.4 到 4.9，Bridge 的网关（另起一个本机的 HTTP 服务，路径、签名、错误码、回答不像样）和分派（`host_sessions/tests/project_rooms.rs`），控制面设备签名的两个地址（`features/project_rooms/tests/device.rs`），应用层本机设备和网页认领是同一个、网络 Agent 403。
- 2026-11-16 3P-3（#904，只有控制面和 Caddy，`/gh/` 页上教人跑的命令要等 3P-4 才有，所以 3P-3、3P-4、3P-5 同一版发）：
  - `GET /gh/{owner}/{repo}` 和 `GET /gh/{owner}/{repo}/badge.svg` 在控制面的 `features/share_page.rs` 里（`share_page/unclaimed.rs` 是“还没人认领”那一页，`share_page/badge.rs` 是徽章），和分享页共用页头、样式、脚本、语言规则和转义。Caddy 在裸域上把 `/gh/*` 和 `/r/*` 一样转给控制面（`render.py`），`APP_PAGE_PATHS` 不加它。
  - 认房间的是 `application` 的 `project_rooms/pages.rs`（`RepositoryRoomPageService`）：先按仓库（不论大小写）找绑定（`ProjectRoomStore::bound_room`，3P-2b 就有），平台冻结、拿掉的、没有分享码的当成没有；再拿分享码问发现页的目录（`detail`，公开着、没关、没拿掉由它再查一遍，在线数也是它那个，`PublicRoomPresenceMatrix`）；两次读之间码换给了别的房间也当成没有。没加 SQL、没加迁移。
  - 页面：名字先照 GitHub 的规矩验，不合的直接 404、不查库。绑着、公开着的出和 `/r/<分享码>` 一字不差的分享页（实况等 3P-6）；别的出同一页“还没人认领”，回 404。和 `/r/` 合起来按来源限流（`AnonymousLimiter`，每个来源每分钟 120 次）、`noindex`、`Cache-Control: public, max-age=60`；语言照分享页：先看 `Accept-Language`，绑着的房间再看房间名里有没有中文，“还没人认领”默认英文。不读 GitHub。
  - 徽章：照设计稿 5，20 像素高，左边黑底白字“Agent Room”带一个小机器人，右边白底黑字“N online”（`?lang=zh-CN` 写“N 在线”），没房间写“no room”（“没有房间”）。不按来源限流；每个仓库的结果在内存里留 60 秒，“没有房间”留 5 分钟；同一个仓库同时只算一次（另起任务算，等着的人走了也算完、记下）；全站每秒最多算 50 个新的；回答带 `Cache-Control: public, max-age=300` 和 `ETag`，认 `If-None-Match`（弱比较、`*`）回 304。图里只有固定的字和算出来的数。
  - 和设计不一样、设计里没写的：
    - 名字不合规矩的不出“还没人认领”，出一页“找不到这个页面”（404、`no-store`），地址里的字不写回页面；徽章也是 404。限流也不算它们。
    - “还没人认领”照设计缓存 60 秒（谁看都一样）；`/r/` 的 404 照旧不缓存。库读不了时和分享页一样回 503“暂时打不开”，不说成没人认领。
    - 那一页第 3 条原来写“谁点开都先看到里面正在说的话”，实况 3P-6 才有，这一版写“房间建好就公开、不加密，谁打开这个地址都进得来”；3P-6a 放实况时改回来。
    - “在房间设置的‘公开与加入’里绑上”写成“公开”：网页设置里那一节就叫“公开”（英文 “Public”）。3P-5 要是改了节名，这里跟着改。
    - 手机上和分享页一样不给下载按钮，换成一句“在电脑上打开这个地址，装好桌面应用再跑命令”。命令和那一句各一个复制按钮；`share.js` 改成把结果写在按钮所在的 `data-copy-scope` 里（分享页只有一个按钮，照旧）。
    - 徽章：在线数问不到（`online` 是空的）时和忙不过来一样只画左边那段，但照常缓存；没问全的写“200+ online”。忙不过来、库读不了的那张 `no-store`、不带 `ETag`、不记在内存里。`ETag` 是图的内容算出来的哈希（标准库的 `DefaultHasher`，不加依赖），样子改了自己就变。`?lang=` 以 `zh` 开头的都算中文。小机器人是 SVG 里画的，不是官网那几张 PNG。
    - 徽章的缓存、每秒的额度都只在这个进程的内存里，开几个控制面各算各的。
  - 测试：控制面 `share_page/repository_tests.rs`（名字不合规矩的不查库、绑着的和 `/r/` 一字不差、“还没人认领”不分哪一种、语言、和 `/r/` 合起来限流、徽章的缓存和 `ETag`/304、“没有房间”留 5 分钟、每秒 50 个以后给不带数的、同一个仓库只算一次），应用层 `project_room_flow.rs` 的“仓库地址只认绑着它、此刻公开着的那间房”，`test_prodops` 里 `/gh/*` 转给控制面、不算应用的页面。
- 2026-11-16 3P-5（#905，只有网页）：
  - 房间设置“公开”一节，房主、公开着的房间在分享链接下面多“GitHub 仓库”一组（`apps/web/src/features/private-rooms/ui/project-room-repository.tsx`）：还没绑时填 `owner/repo` 点“绑上”；在等时写着在等哪个仓库、那两行（README 徽章、`AGENTS.md` 那一行）各一个复制按钮、上次核对的结果，“现在核对”“不绑了”；绑上了写着仓库（点了在新标签页打开 GitHub）、谁什么时候核对上的、在哪个文件里，“再核对一次”“解绑”。错误码都换成大白话，错误码和关联 ID 收进详情。
  - 顶栏房间名旁边的勾（`apps/web/src/features/lobby/ui/repository-mark.tsx`），指上去和读屏读的都是“已验证的 GitHub 仓库 owner/repo”，点了在新标签页打开 GitHub。
  - 限额的说法中英文都改成“不绑仓库的公开房间每个账户只能有一间”：新建、设成公开、转让、解绑四处。`room_listing.repository_bound` 有了说法：绑着仓库的房间不能换分享码，链接写在仓库里。
  - 客户端 `ControlPlaneProjectRoomClient` 经组合根注入的 fetch 发请求，七个接口的回答都是新的 schema（`.strict()`）。429 的秒数放进 `PrivateRoomFailure.retryAfterSeconds`（回答里的 `retryAfterSeconds`，没有就看 `Retry-After`），私人房间的客户端共用这一段。
  - 和设计不一样、设计里没写的：
    - 设计里叫“公开与加入”的那一节，网页上一直叫“公开”（Public）。这一组就放在这一节里，节名没改：3P-3 的 `/gh/` 页教人去“房间设置 → 公开”绑仓库，改名得两边一起改。
    - 勾只给房主看。`GET /private-rooms/{c}/repository` 只有房主读得到，别人问回 403；别的回答又不能加字段（`.strict()`），所以别的成员此刻读不到房间绑着哪个仓库。要让大家都看到，得另给一个谁都能读的接口，这一步没改服务器。
    - 在等的认领记在这个浏览器里（`localStorage`，按账户和房间）：服务器没有“这间房在等哪个认领”的接口。换个浏览器看到的是“还没绑”，再填同一个仓库，服务器回的还是同一个认领，不多占名额。设置开着时每 30 秒问一次认领，后台核对上了就换成“绑上了”。
    - 那两行在等的整段时间都显示，不只是刚点“绑上”的那一下：刷新以后也要拿得到。
    - 解绑先确认一次（说清楚仓库的地址不再指到这里、勾没了、房间照旧公开）；设计里没写。
    - 绑着仓库的房间，“分享链接”下面不给“换一个分享码”，换成那一句说法；服务器回 409 时也说这一句。
    - 输入框也认从浏览器地址栏贴过来的 `https://github.com/owner/repo`（去掉 `.git` 和末尾的斜杠），发请求前按控制面同一条规矩先验一遍，不合规矩就地说怎么写。
    - “谁核对上的”只分“你”和“另一个账户”（换了房主以后），另一个账户的 ID 收进详情，不去查名字。
    - 勾在场景版面的房间名旁边也有，不只是对话版面的顶栏。
    - 设计“接口”里列的 `project_room.not_in_repository`、`project_room.github_unavailable` 在 3P-2b 里是核对的结果，不是错误码：网页照结果说（“仓库里还没有这一行”“暂时读不到 GitHub”等）。
    - 3P-2b 留给这一步定的“仓库被别人要走时原来那间房里照实说”没做：要走以后绑定就删了，网页只看得到“还没绑”，服务器不记“被要走过”。要说得服务器先记下来，放到以后。
  - 测试：客户端的 schema（多字段、少字段、不认得的结果、不是 github.com 的地址都当回答无效）、429 和 `Retry-After`、解绑的 409；这一组的三种样子、复制按钮、太快时数秒、认领没了、解绑超名额；只有房主、只在公开着的房间里有；绑着时不给换码；顶栏的勾。浏览器验收 `e2e/project-rooms.e2e.ts` 在 1440 和 390 宽走三种样子和勾，带 axe 检查，夹具 `lobby-scene.html?private&listed&repository=none|pending|bound`（`&repositoryLimit` 解绑回 409）。
- 2026-11-16 3P-4b（#906，命令行 `agent-room project`，还没有本机 MCP 的工具）：
  - 在仓库目录里跑：问 git 仓库根目录（`git rev-parse --show-toplevel`）和 `origin` 的地址，认出 `owner/repo`；经 Bridge 认领（IPC 4.9 的 `ClaimRepository`），把那一行写进仓库根目录的 `AGENTS.md`，回 README 徽章那一行和下一步（`next`：提交、推上去，一般几分钟内认上）。不替人 `git commit`、不推送。
  - 回答照别的命令一样是一份 JSON：`repository`、`status`、`shareUrl`、`agentsLine`、`readmeBadge`，在等的有 `claimId`、`lastCheck`，认上了的有 `catalogId`，写文件的结果在 `agentsFile`（`created`、`appended`、`already_linked` 带 `link` 和 `sameRoom`，不在 git 仓库里是 `skipped`）。
  - 认 `origin` 只认 github.com 的 https 和 ssh 两种写法：`https://github.com/o/r(.git)`（可以带账号令牌、`:443`）、`git@github.com:o/r(.git)`、`ssh://git@github.com(:22)/o/r(.git)`；地址里的账号令牌不留下。别的托管、`http://`、`git://`、本机路径都认不出，要 `--repo owner/repo`。认的规矩放在 `agent-client` 的 `project_rooms.rs`（`repository_from_remote`），3P-4c 的本机 MCP 也用这一份里的认领和等待。
  - `--wait`：先核对一次，之后每分钟一次，最多 15 分钟（`wait_until_bound`）。核对太快就等下一分钟。核对说没有这个认领（后台核对认上以后认领就删了）就用同样的仓库和房间再认领一次，认上了回那间房。认上以后照 `join --room <分享链接>` 进房间，回答是 `join` 的那一份，另带 `project`（认领的那一份）。
  - 再跑一次同一条命令：控制面回同一个认领，`AGENTS.md` 里已经有这一行就不再写（`already_linked`），接着核对同一个认领。仓库已经绑了房间的只回那间房的分享链接，不再写文件，带 `--wait` 时进去。
  - `--room <房间名或房间号>` 照 `rooms` 的名单换成房间号，绑你名下那间公开房间；能不能绑由控制面把关。
  - `guide` 加了一节 `project`，开头就是设计里那一句（`To give the repository you are working in a room, run agent-room project when the person asks.`），`commands` 里加了 `project`、`project --wait --name <your name>`。
  - 和设计不一样、设计里没写的：
    - 多了 `--name`（只能和 `--wait` 一起用）：认上以后进房间时 Agent 的名字，和 `join --name` 一样。
    - 不在 git 仓库里（比如只给了 `--repo`）照样认领，但不往哪里写 `AGENTS.md`，照实说让人自己加。
    - “里面已经有 Agent Room 的链接”只认这个站点的 `<站点>/r/…`，回那个链接和是不是这间房的。加在已有文件末尾时前面空一行，换行照文件原来的（`\r\n` 或 `\n`）。
    - 等的时候原来的认领作废了（7 天没认上），再认领拿到的是新码：停下，说要把仓库里那一行换成新的，不接着等。
    - 15 分钟里最多核对 15 次（第一次马上）；到时间还没认上就回原来的认领，说再跑一次接着等。
    - `agents.md`、`agents.txt` 那一句照“分步”放在 3P-6a，这一步没改。
  - 测试：`agent-client` 认 `origin` 的写法、每分钟核对、太快等下一分钟、认领被后台认上后再认领、作废换了码、别的拒绝（暂停时间跑）；命令行认仓库、`AGENTS.md` 新建、加在末尾（换行、空行）、已经有链接、再跑一次接着等同一个认领而且只写一行、`--room` 换房间号、不在 git 仓库里、控制面拒绝和 Bridge 太旧；命令进程里 `--name` 不带 `--wait`、坏的 `--repo` 在碰 Bridge 之前就报错，`guide` 里有那一句。命令行的测试在设了和没设 `CLAUDE_CODE_SESSION_ID` 两种环境下都跑过。
- 2026-11-16 3P-4c（#907，本机 MCP `agent_room_claim_repository`）：
  - 参数 `repo`、`room`、`wait`，和命令行一样。工具不碰文件：认领（`wait` 时接着每分钟核对一次、最多 15 分钟），回 `repository`、`status`、`shareUrl`、`agentsLine`、`readmeBadge`、`next`，在等的另带 `claimId`、`lastCheck`，认上了的带 `catalogId`。`next` 说下一步：没认上时叫 Agent 把那一行写进仓库根目录的 `AGENTS.md`（没有就建，里面已经有 Agent Room 的链接就别动、告诉人）、把徽章给人看、问人要不要提交推送，不许自己提交推送；认上了叫它用 `agent_room_join` 按 `shareUrl` 进房间。
  - 认领、核对和等待用 `agent-client` 的 `project_rooms.rs`，和命令行同一份；`room` 照 `agent_room_list_rooms` 的名单换成房间号（和建子房间找父房间一样）。不在会话里：认领是替这台电脑的账户做的。
  - 风险提示是“不只读、收得回来、重做不多出东西、碰外面”（`(False, False, True, True)`）：同一个人同一个仓库再认领，控制面回同一个认领。
  - 设计里那句英文没放进服务说明：服务说明已经 1532 字节，加上就超过 1536。照设计写在工具说明的开头（`To give the repository you are working in a room, call this when the user asks, …`）。
  - 和设计不一样、设计里没写的：
    - `repo` 必填：MCP 服务是宿主拉起的，不知道 Agent 在哪个目录里干活，所以不替它读 `origin`，由 Agent 自己读 `git remote get-url origin` 传进来。不像 `owner/repo` 的在碰 Bridge 之前就拒绝（`bridge.ipc.repository_invalid`）。
    - 认上以后不替 Agent 进房间，只在 `next` 里叫它用 `agent_room_join`：进房间要开会话、挑人物，交给接入那个工具做。
    - 等的时候原来的认领作废了、再认领拿到新码时，和命令行一样停下，`next` 说要把仓库里那一行和徽章换成新的。
  - 测试：只认领不核对、回那一行和徽章，坏的仓库名不碰 Bridge，`room` 换房间号、找不到的房间不认领，`wait` 每分钟核对一次认上了说怎么进、等满还没认上，已经绑了的不核对，控制面拒绝和旧版 Bridge 的说法（暂停时间跑）；列全部工具、风险提示和 schema 的测试，发版门禁（`EXPECTED_TOOL_ANNOTATIONS`）和 `tools/mcp_client.py` 的 schema 校验都加上了它，语言测试也查 `project_rooms.rs`。MCP 的测试在设了和没设 `CLAUDE_CODE_SESSION_ID` 两种环境下都跑过。
- 2026-11-16 3P-5b（#908，补 3P-5 留下的两处，控制面和网页）：
  - 勾给房间里的每个人看：新加 `GET /rooms/{c}/repository`，谁登录了都能问，只回 `{ repository, repositoryUrl }`。只认公开着、没关、平台没拿掉、有分享码的主房间（和 `/gh/` 页认房间的规矩一样，`application` 的 `project_rooms/bindings.rs` 的 `room_repository_internal`），别的一律 `null`，不分是没有这间房、没绑还是不公开了，免得拿它探私人房间在不在。公开着的房间绑着哪个仓库本来就在 `/gh/` 上公开着，所以不查是不是房间里的人；谁什么时候在哪个文件里核对上的照旧只在房主的接口里。网页只在公开着的房间里问（`useRoomRepositoryMark`），挂在房主那一份的键下面，设置里绑上、解绑以后一起重问。
  - 在等的认领问服务器：新加 `GET /project-rooms/claims`，列自己在等的认领（库里本来就有 `claims_of`）。设置里那一组找要绑这间房的那一个（一间房同时只能有一个在等：码是这间房的，`insert_claim` 不让两个认领用同一个码），换个浏览器、换台电脑也接得上。3P-5 记在浏览器里的那一套（`rememberClaim`）去掉了：3P-5 还没发版，没有人的浏览器里存着它，不用清。
  - 隐私说明页补上 3P-2b 留给 3P-3、3P-5 的两句：房间公开期间绑着哪个仓库谁都看得到（`/gh/` 地址和房间名旁边的勾）；README 徽章由 GitHub 的图片代理来取，我们看不到看 README 的人。
  - 3P-5 那一条里“勾只给房主看”“在等的认领记在这个浏览器里”两样从这一步起不再是这样。“仓库被别人要走时原来那间房里照实说”照旧没做：要服务器先记下“被要走过”。
  - 测试：`application` 的流程测试（列认领只有自己的、认上了就没了；谁都能问、只认公开着的主房间，改回私密、关掉、平台拿掉、没有分享码都是 `None`）；控制面 `features/project_rooms/tests/reading.rs`（两个接口的回答、`null`、不登录 401）；网页客户端的 schema（列表里每一个照认领严格校验、勾只认 github.com 的 https 地址、多字段无效）、设置里那一组（在等的认领从列表里找、别的房间的不算、问不到能重试）、浏览器验收 `e2e/project-rooms.e2e.ts` 加一条：不是房主（`&asAdmin`）也看得到勾，“GitHub 仓库”一组照旧只给房主。
- 2026-11-16 3P-9a（无头验收的前一半：认领、`/gh/` 页、徽章、命令行和本机 MCP）：
  - 3P-9 拆成两步：3P-9a 只走 3P-2 到 3P-5 已经有的；实况、一段对话和私人房间、加密的公开房间、要申请的房间读不到这几条，等 3P-6、3P-7 合了放在 3P-9b。
  - 控制面加了一项配置 `AGENT_ROOM_ACCEPTANCE_GITHUB_ADDRESS`：只认本机回环地址加端口（`127.0.0.1:18766`、`[::1]:18766`），别的地址、域名、带 `http://` 的、端口 0 一律启动失败。配了它读取器就连这个地址上的假 GitHub（明文 HTTP，不查地址：回环地址本来就过不了那一关），启动时记一条告警。没配就是写死的 `raw.githubusercontent.com`。原来只给单元测试的 `#[cfg(test)]` 构造换成了 `RawEndpoint::loopback`，不是回环地址就给不出来。生产的部署配置和 `tools/prodops` 里不许出现这个名字，`test_prodops` 卡着。
  - 纵向验收的控制面（`vertical_control_plane_environment`）带上它，端口 `FAKE_GITHUB_PORT`。这一轮在 `tools/vertical_project_rooms.py`，排在“公开房间改成公开、读不到以前的话”那一轮之后，用验收账号的发送方 Bridge：
    - 一个只有 `origin` 的 git 仓库里跑 `agent-room project`：等着的认领，那一行和徽章都指着同一个 `/r/<码>`，新建了 `AGENTS.md`；再跑一次是同一个认领，不再写（`already_linked`）。
    - 假 GitHub 放上命令行写的 `AGENTS.md`，本机 MCP `agent_room_claim_repository` 带 `wait`：第一次核对就认上，分享链接还是那一个，`next` 叫它用 `agent_room_join`；按分享链接进去说一句。
    - 库里认上以后建的房间叫 `owner/repo`、公开、不加密、绑着这个仓库；认上了的仓库再跑命令行直接回这间房、不碰 `AGENTS.md`。
    - `/gh/owner/repo`（owner 换成大写也行）是这间房的分享页；徽章写着在线数，`?lang=zh-CN` 写“N 在线”。没人认领的仓库出“还没人认领”（404），徽章写“no room”“没有房间”；名字不合规矩的 404。
    - 假 GitHub 只被读过这个仓库的 `AGENTS.md`、`README.md`；看 `/gh/` 页和徽章前后，它收到的请求一个没多。
  - 没人认领的那几条用另一个仓库名：“没有房间”的徽章在内存里留 5 分钟，先看了要认的那个仓库的徽章，认上以后 5 分钟内还是“没有房间”。
  - 测试：控制面配置只认回环地址、读取器 `loopback` 不给别的地址；假 GitHub 和认领那一行的检查有 Python 单元测试（`tools/tests/test_vertical_project_rooms.py`），整轮在 PR 分支上派发 `suite=all` 跑。
- 2026-11-16 3P-10a（官网、README、指南的前一半）：
  - 3P-10 拆成两步：这一步只写已经合了的（认领、那一行、徽章、`/gh/` 页、命令行、本机 MCP、网页上绑已有的房间），实况、一段对话、预览图等它们做好再补（3P-10b）。不然指南里写着的东西发版时还没有。
  - 新指南 `apps/web/site/guides/project-room.<en|zh-CN>.md`（“Give your GitHub project a room”“给你的 GitHub 项目建一间房”），排在第 4 篇，“名字”那篇挪到第 5。照设计点名了 GitHub、Claude Code、Codex。三种做法：命令行、跟本机 Agent 说一声、网页上绑已有的房间；推上去以后怎么认上、认上以后的房间什么样、项目换了主人、谁读得到什么、遇到问题。指南里引的界面文字和报错照网页现在的中英文写。
  - 首页“Agent 从哪扇门进来”三扇门下面加一句，链到这篇（`ways.project`，`<p class="band__note">`；这一种语言的这篇没生成时不写，链接不落空）；`llms.txt` 加一条 `/gh/owner/repo` 和 `agent-room project`。
  - README 中英文：“能拿它做什么”加一条项目房间，“谁能读到什么”加一条项目房间（不加密；只在核对时读默认分支的 `AGENTS.md`、`README.md`；看 `/gh/` 页和徽章不读 GitHub；徽章经 GitHub 的图片代理）。
  - 和设计不一样、设计里没写的：命令行认领的仓库（不带 `--room`）没有房间，网页上哪间房的设置里都看不到它，所以指南里“已经有 3 个在等”只说推上去或者等 7 天作废，给已有房间的认领才能在设置里“不绑了”。
- 2026-11-16 3P-6a-1（#909，只有控制面和隐私说明页；分享页还不放，按房间开的快照、`/r/<码>/live`、页面、内容安全策略、`agents.txt` 那一句是 3P-6a-2）：
  - 迁移 `202611160006_room_live_view.sql` 建 `room_live_view`：一间房一行，谁、什么时候打开的。关就是删这一行，再开是新的一刻。
  - 开关：`GET`、`PUT /private-rooms/{c}/live-view`（`features/room_live_view.rs`，用例在 `application` 的 `room_live.rs`）。回答是新的：`{ enabled, enabledAtUnixMs, live, unmetConditions, canChange }`，房间里已加入的人都读得到，`canChange` 是问的人有没有“管理”。`unmetConditions` 按先后是 `sub_room`、`closed`、`not_listed`、`platform_hold`、`encrypted`、`join_mode`，`live` 是开着而且一样不差。开不了回 409 `room_live.not_eligible`，`details.condition` 是排在最前的那一样；别的错误码：`room_live.forbidden`、`not_found`、`conflict`、`dependency_unavailable`、`internal`、`invalid`、`invalid_origin`、`invalid_resource`。说法是中文，网页按错误码换（3P-6b）。
  - 打开：先照目录上的标记查一遍几样，再问一次聊天服务器房间状态里有没有 `m.room.encryption`（`RoomEncryptionState`，有这一条就当加密，问不到回 503），最后库里一句 `INSERT … SELECT` 锁着房间目录和公开设置那两行（`FOR SHARE`）再查一遍，连“不是子房间”一起。已经开着的再开不换人、不换那一刻，也不再问聊天服务器。那一句没记下、再读又都满足时回 409 `room_live.conflict`。
  - 自动关：改成要申请或只能邀请、改回私密（`save_listing`）、目录记成加密（`mark_room_encrypted`）、归档或关掉（`write_room_rows`：房主归档、网络 Agent 关房、停用时系统关、房主离开）、删账户（`archive_private_rooms`）都在改房间的那个事务里、改完那几行以后删这一行（`rooms/live_views.rs` 的 `turn_off_live_view`），和打开那一句抢同两行的锁。
  - 默认：项目房间建房的那个事务里就记上（认领的人、建房那一刻）。以前就公开着的房间没有这一行，就是关着。
  - 和设计不一样、设计里没写的：
    - 设计条件里写“房主开着”，接口一节写“有‘管理’的人能改”：照接口，房主和管理员都能开关。归档了的房间也关得掉，开不了。
    - 读的时候只看目录上的加密标记，不问聊天服务器；只在打开的那一下问。3P-6a-2 的快照每 30 秒读房间状态时再看一遍，看到 `m.room.encryption` 就不放。
    - 平台冻结、拿掉不删那一行（和仓库绑定一样），只是不放（`unmetConditions` 里有 `platform_hold`）；解除以后接着放，不用房主再开。
    - 房间转让不关。
    - 新建公开房间时的勾（3P-6b）不往建房的请求里加字段：建好以后网页再 `PUT` 一次。
    - “下载我的数据”加了 `liveViews`（他打开、还开着的：哪间房、什么时候）。
    - 隐私说明页在这一步就照两步都做完的样子改了：存开关的表在这一步，按规矩说明页跟着同一个 PR 改。3P-6a-2 要是没赶上同一版，页面上说的实况就还没真放出来，只多说、不少说。
  - 测试：应用层流程（`application/tests/room_live_flow.rs`）、接口（`features/room_live_view/tests.rs`）、问聊天服务器加没加密（`matrix-provisioning-adapter` 的 `room_listing.rs`）、隐私说明页的说法（`privacy-page.spec.tsx`）。真实数据库的（`postgres-adapter` 的 `private_rooms/live_views.rs`，`repository_claims.rs` 建项目房间那一条，`real_database.rs` 删账户那一条和表的名单）只在派发 `suite=all` 时跑。
- 2026-11-16 3P-6a-2（#912，分享页真放出来，控制面和 Caddy；网页的开关、建房时的勾、顶栏那一行小字是 3P-6b）：
  - `GET /r/{分享码}/live` 在 `features/share_page.rs`；快照是 `public_watch/rooms.rs` 的 `RoomLiveWatch`，拼成什么样在 `public_watch/live_view.rs`。按分享码找房间是新的 `LiveRoomLookup`（`postgres-adapter` 的 `rooms/live_views.rs`），一句 SQL 查齐：公开着、直接进、私人房间、在用、目录上不加密、实例在用、平台没拿掉、不是子房间、开着实况，回房间、Matrix 房间号和开的那一刻。
  - 和围观公开大厅共用：解析和验签（`reading::verified_messages`）、隐藏的消息（`reading::hidden_events`，只认建房间的应用服务账号写的）、编号（`view::Keys`，两边各一份盐）、哪些是网络 Agent（拆成 `public_watch/agent_kinds.rs`，两边各记一份）。消息 3 秒一读、旧快照最多再给 1 分钟、一次最多等 8 秒，和围观一样。
  - 按房间：库里查到的放不放在内存里留 5 秒（最多记 4096 个码，满了先清过期的）；不放了、换了一刻，原来那份快照扔掉。快照按“房间、开的那一刻”存，最多 200 间，久没人看的先扔；同时重读的最多 4 间，排不上的先给旧的。房间状态 30 秒读一次：隐藏了哪些、在线数（目录 `detail` 那一份，和卡片上同一个数；目录说没有这间房了就当不放）、说话的本机 Agent 是谁的（`RoomPeopleDirectory::agent_owners`，只问还不知道的，读房间状态时清掉重问）。状态里有 `m.room.encryption`（`state_key` 为空）就封住这一份：以后都回 404、不再读，直到换了一刻。
  - 回答：`{ schemaVersion: 1, online: { count, atLeast } | null, participants: [{ key, name, kind, owner }], messages: [...], updatedAtUnixMs }`。消息和围观的一样（`key`、`author`、`text`、`truncated`、`withheld`、`attachment`、`edited`、`replyTo`、`sentAtUnixMs`），只给开的那一刻以后最近 20 句，每句 280 个字。名单只有这 20 句里说过话的，新说话的在前：人写昵称（有 @ 的只留前面那段），本机 Agent 带主人，网络 Agent 不带；不列在场的。`Cache-Control: public, max-age=3`、`noindex`。不放、没有这个码、码写得不对一律 404 `room_live.unavailable`；读不到又没有不太旧的回 503 `room_live.dependency_unavailable` 带 `Retry-After: 5`；来源名额用完回 429 `room_live.rate_limited`。
  - 限流和分享页同一份（每个来源每分钟 120 次），只在 5 秒内没人问过这个码时才算：开着页面的人每 5 秒问一次，别让他把自己的名额用完；挨个猜码的每次都是没问过的码。
  - 页面：开着实况的房间（`/r/<码>` 和绑了仓库的 `/gh/owner/repo` 一样）在房间卡片下面多一块“房间里正在说的话”，先藏着，`share.js` 头一次问到 `/live` 才露出来；回 404 就藏起来、不再问；页面在后台时不问。没有脚本时只有卡片。消息一律当纯文本放（`textContent`），要用的字在那一块的 `data-` 属性里。只有直接进、不加密的房间才去问放不放。页面照旧缓存 60 秒，开了以后最多一分钟页面上才多出那一块；关了以后最多 5 秒 `/live` 就回 404，那一块跟着藏起来。
  - 内容安全策略：分享页、仓库地址和它们的样式、脚本、徽章由控制面自己带一份（`share_page.rs` 的 `PAGE_POLICY`，比网站那一份多一句 `connect-src 'self'`），Caddy 裸域那一份不再给 `/r/*`、`/gh/*`（`@site_csp not path /r/* /gh/*`），不然浏览器两份都照着办，脚本问不了 `/live`。改了 `render.py`，发版实机验收要设备码。
  - `/gh/` 没人认领那一页第 3 条改回“房间建好就公开、不加密，谁点开都先看到里面正在说的话”。
  - `agents.md`（`agents.txt` 是同一份）的英文摘要和“进公开房间”各加一句：仓库的 `AGENTS.md` 或 README 里写着 Agent Room 链接的，就是这个项目的房间；公开房间可能开着实况，开着时在里面说的话，不登录的人在分享页上也看得到。
  - 和设计不一样、设计里没写的：
    - 那一块的标题写“房间里正在说的话”（英文 “What’s being said”）；时间按看的人的浏览器写几点几分；回复的是哪一句没写出来（快照里有 `replyTo`）。
    - 目录标记和聊天服务器对不上（目录说不加密、聊天服务器上有 `m.room.encryption`）时，最多 30 秒认出来，记一条告警。经我们自己加密的路（`mark_room_encrypted`）在改标记的事务里就删了开关，5 秒内停。
    - 隐私说明页 3P-6a-1 就照两步做完的样子改好了，这一步不再改页面，只在说明页的设计文档里补上依据。
  - 测试：快照（`public_watch/tests/live_rooms.rs`：只给开的那一刻以后最近 20 句、每句 280 个字，名字和主人，隐藏和撤回，不放的码 5 秒内不再查，关了 5 秒内停、再开是新的一刻，聊天服务器上加了密就封住，目录说不公开了就不放，3 秒、30 秒的读法，旧快照最多 1 分钟，最多 200 间），页面和接口（`features/share_page/live_tests.rs`：那一块在哪、什么时候不去问，`/live` 的回答、404、503、429 和缓存，这几页都只带一份控制面的内容安全策略），Caddy（`tools/tests/test_prodops.py`）。真实数据库的（`private_rooms/live_views.rs` 的“分享码只认此刻放着实况的主房间”）只在派发 `suite=all` 时跑。
- 2026-11-16 3P-6b（#913，网页：设置里的开关、新建公开房间时的勾、顶栏那一小句）：
  - 接口的客户端 `private-rooms/adapters/control-plane-room-live-view-client.ts`，回答照 `{ enabled, enabledAtUnixMs, live, unmetConditions, canChange }` 严格校验（`domain/room-live-view.ts`）。409 的 `details.condition` 由共用的 `PrivateRoomHttp` 读进 `PrivateRoomFailure.condition`，别的接口用不着就是空的。查询键挂在私人房间列表的键下面（`roomLiveViewQueryKey`），公开设置存好以后原来就有的那次 `invalidateQueries` 把它一起重读：改成要申请以后开关跟着变成关、灰着。
  - 房间设置“公开”一节，分享链接下面、“GitHub 仓库”上面多“分享页放实况”一组（`ui/room-live-view-settings.tsx`）：说明一句；开关（`role="switch"`）下面写此刻的样子（“… 起开着，只放那以后说的话”“开着，但分享页上现在没在放”“关着”）。关着、又有一样不满足时开关灰着，下面一条一条说差哪一样、怎么改。打开先在下面出确认，说清楚以后说的话不登录的人都看得到、只放最近 20 句、改加入方式或改回私密会自己关；确认之前开关照旧是关着。关掉不用确认。每个 `room_live.*` 错误码都有中英文的一句，409 照 `condition` 说是哪一样；说完重读一次。
  - 新建房间的对话框，公开、直接进时多一个“分享页放实况”的勾，默认勾上。建房的请求里不加字段：建好、加入以后网页再 `PUT` 一次，开成了才跳进房间。没开成时房间照样在：不再建，说一句“房间建好了，实况没开，可以在设置里开”，按钮换成“进房间”。为此 `useRoomCommands().create` 只建、加入，不跳过去，跳进房间是新的 `show`。
  - 已有的房间设成公开时，确认框（不加密、直接进时）多一句：实况不会自己打开，设成公开以后在这一节里开。
  - 顶栏：开着而且正在放时（`live`），房间名旁边那一小句换成“公开房间 · 不登录也看得到这里说的话”，指上去说整句；手机上只剩后一半。走的是 `RoomKindLabel` 的 `live`，和公共大厅、不加密那几句同一个地方，排在归档后面、“服务器读得到”和“不加密”前面。只在公开着的房间里问 `GET`，公共大厅、私聊、私密的房间不问。
  - 浏览器验收：夹具 `lobby-scene.html?private&listed&live=on|off|ineligible`（`ineligible` 是要申请的房间；`&asAdmin` 是不是房主的管理员），`e2e/room-live-view.e2e.ts`，1440 和 390 两种宽度、带 axe。夹具和服务器一样，公开设置不再是直接进时把实况关掉。
  - 和设计不一样、设计里没写的：
    - 设计写“房主开着”，3P-6a-1 照接口让管理员也能改：这一组房间里的人都看得到（公开设置照旧只有房主改），开关只给 `canChange` 的人。
    - 公开设置那句“不登录看得到什么”原来说“房间里说的话不给”，改成“开了分享页放实况的除外”。
    - 已有房间设成公开的确认框不写“不登录也看得到这里说的话”：那一刻实况还关着，只说可以去开；打开实况时的确认框才说看得到什么。
    - 新建时的勾只在直接进的公开房间出：要申请、只能邀请的开不了。
- 2026-11-16 3P-9b-1（无头验收：分享页的实况和读不到的几条路）：
  - 3P-9b 拆成两步：这一步只走 3P-6 已经有的实况和读不到的几条路；一段对话等 3P-7 合了放在 3P-9b-2。
  - 接在 3P-9a 那一轮后面（`tools/vertical_project_rooms.py` 的 `verify_live_view`），本机 MCP 的会话一直开到这一段做完。网页一侧是新的 `apps/web/e2e-vertical/project-room-live.e2e.ts`：验收账号（项目房间的房主）在真实网页上登录，和这边靠五个文件对步子。
    - 不登录读 `/r/<码>/live`：Agent 进房间说的那句在，说话人是“Project helper #编号”、本机 Agent、主人是验收账号；`/gh/owner/repo` 页上有实况那一块（`data-live`）。
    - 房主在房间设置“公开”一节里关掉（开关下面原来写着“… 起开着”，关掉写“关着”，顶栏那一小句没了）：`/live` 回 404 `room_live.unavailable`，`/gh/` 页上那一块没了。关着时 Agent 说一句。
    - 房主再打开（在下面确认）：顶栏那一小句回来了。Agent 再说一句：实况里有这句，打开之前的两句（第一次开着时说的、关着时说的）都不在。
    - 房主经控制面的接口把项目房间改成要申请：实况自己关了（`enabled` 是 false，差的是 `join_mode`），再开回 409 `join_mode`；`/live` 回 404，`/gh/` 页还是这间房、没有那一块。改回直接进，实况不自己回来。
    - 公开房间那一轮验收账号建的、最后改回私密的房间：私密时开不了（409 `not_listed`，差的里面也有 `encrypted`），它记着的分享码的 `/live` 回 404。再设成公开、直接进，就是加密的公开房间：差的只有 `encrypted`，开回 409 `encrypted`；分享页 200、没有实况那一块，`/live` 还是 404。看完改回私密。这两间房不登录的读法在网页用例里用不带登录的请求做。
    - 最后假 GitHub 收到的请求还是一个没多：看分享页、实况都不读 GitHub。
  - 要申请、私密、加密的公开房间都用已有的房间：项目房间自己改加入方式，公开房间那一轮的房间改回公开。没另建房间，也就碰不上每个账户 1 间不绑仓库的公开房间的限额（项目房间不算）。
  - 关掉以后 `/live` 最多再放 5 秒（按分享码查到的“放”在内存里留 5 秒），所以这边轮询到 404 为止（最多 30 秒），别只查一次；再打开以后同理，轮询到那一句出来（最多 90 秒）。网页那边故意去开、开不了的 409，`collectUnhandledFailures` 只放过这两间房的 `live-view`。
  - 没走的：一段对话（3P-9b-2）；被平台拿掉的房间（要平台管理员）；子房间、只能邀请的房间（和这几样在同一个判断里，`application` 的 `tests/room_live_flow.rs` 有）；目录上没记加密、聊天服务器上却有 `m.room.encryption` 时快照封住（要在库里改目录，`public_watch/tests/live_rooms.rs` 有）；不是房主的管理员开关；归档、删账户时自动关。
  - 测试：读实况、按编号找说话人、认 Agent 那一句、看分享页有没有那一块、网页用例要的变量都给了，有 Python 单元测试（`tools/tests/test_vertical_project_rooms.py`）；整轮在 PR 分支上派发 `suite=all` 跑。
