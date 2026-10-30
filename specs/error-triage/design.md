# 生产上的问题自动汇总、开 issue、叫醒开发 Agent 处理：设计

2026-10-24 维护者让查三个新注册用户用得正不正常，编码 Agent 手动翻了生产库和日志，找到两处用户碰上、却没人知道的问题：打开私人房间的地址回 500（#570），隔天点过期的验证链接以后登录失败。维护者接着问：有没有专门的错误监控？查到了要不要自动在 GitHub 开 issue？开发的 Agent 能不能定期看 issue 来处理？

这份文档回答这三个问题，按步交付。

## 背景

查于 2026-10-24。

- **生产没开监控。** `telemetry.enabled` 是 false，Prometheus、Grafana、Alertmanager 的规则和仪表盘都写好了（`infra/production/observability-rules.yaml`、`docs/operations/observability.md`），但没在跑，没有任何自动告警。
- **就算开了也抓不到这次的两个问题。** 告警盯的是可用率、延迟、积压这些总量。这次的 500 一共一次，登录失败两次，都远在告警线以下。用户碰上的多半是这种“偶尔一次、正好卡住一个人”的错。
- **线索都在日志里，但日志留不住。**
  - 控制面是结构化 JSON：级别、模块、消息、错误码、路径、关联号都有。
  - 身份服务（Keycloak）的事件（`LOGIN_ERROR`、`VERIFY_EMAIL_ERROR` 等）只写日志不进库（realm 的 `events_enabled` 是 false），行里带邮箱和 IP。
  - Synapse 的访问日志有状态码和路径，也带 IP。
  - 容器日志写满就覆盖（`compose.yaml` 的 `x-container-logging`），发版重建的容器从部署那一刻重新记。10-24 查的时候控制面只回看到 10-22，Synapse 只有三天半。
- **用户不会来报。** 三个人碰到问题都没联系我们，有一位登录失败以后再没回来。
- **开发在云端。** 云端的编码 Agent 能读写仓库、开 PR，但连不上生产（SSH 和生产日志只在维护者的 Windows 机器上）。

## 怎么做

分四层：服务器每小时自己汇总一次，有新问题就开或更新 issue，每天出一份新用户体检，有新 issue 就叫醒开发 Agent 处理。

### 1. 服务器每小时汇总一次错误

- 新加 `tools/prodops/error_digest.py`，入口是 `production.py error-digest`，由 systemd 定时器每小时跑一次（和备份定时器一样，由 `tools/prodops/schedule.py` 装）。只读：读上一小时的容器日志，加几条只读 SQL，不改任何东西，一次几秒，不常驻。
- 每条错误算一个**指纹**，同一种错只算一种：
  - 控制面的 WARN、ERROR：模块 + 消息 + 错误码（或 `failure`）+ 路由模板。路径里的 UUID、Matrix ID、房间号换成占位，比如 `POST /lobbies/{id}/entry`。
  - 控制面回的 5xx：方法 + 路由模板 + 状态码。
  - 身份服务的 `*_ERROR` 事件：事件类型 + `error`。`reason` 里只留固定的那部分，这次的 `Invalid scopes: openidprofile` 这种不含个人信息的照留。
  - Synapse 的 5xx，以及“已知正常”之外的 4xx：方法 + 路径模板 + 状态码。
  - 库里卡住的后台活，只读查：建房任务失败、删账户任务失败、治理到期撤不掉、备份超时没做。
- 每个指纹记：第一次和最近一次出现的时间、次数、涉及几个账户（只是一个数，不存是谁）、当时的版本号、一条打码后的样例、关联号。凭关联号能在服务器日志覆盖前找回原始行。
- **已知正常的不报**：仓库里放一份噪音表 `tools/prodops/error_signatures.toml`。Agent 第一次读不到的 404、网络 Agent 报在线状态偶尔的 429、Synapse 的 `sentinel context` 之类都写在里面，每条带一句为什么正常。开发 Agent 判断某个新指纹是正常现象时，改的就是这份表，走 PR。
- 服务器上记一份状态文件（`<状态目录>/error-digest/state.json`），只存指纹、次数和对应的 issue 号。

### 2. 有新问题就开 issue，同一种错只开一个

- issue 开在私有仓库 [`rainyflash/agent-room-ops`](https://github.com/rainyflash/agent-room-ops)（2026-10-24 建好，标签也建好了），修复的 PR 照旧开在公开仓库。
- 新指纹开一个 issue，标签 `prod-error`，再按服务（`control-plane`、`identity`、`synapse`）和轻重（`p1` 有人被挡住了、`p2` 出错但能绕过、`p3` 只是日志难看）各加一个标签。轻重先按规则定：5xx 和登录失败算 p1，其余 p2，开发 Agent 看过可以改。
- 已经开着的指纹又出现：在原 issue 下补一条评论，每天最多一条，写这一天的次数和账户数。
- 已经关掉的指纹又出现：重新打开，加 `regression` 标签，写“在 vX 上线以后又出现”。
- 防刷屏：每次汇总最多开 5 个新 issue，多出来的合进当天一条“今天还有这些”的 issue。
- 令牌：服务器上放一个 GitHub fine-grained token，只给 `agent-room-ops` 的 Issues 读写，366 天过期，放在 `/etc/agent-room/ops-issues.token`（root、0600）。不放进 `<状态目录>/secrets`，那里由 `SecretStore` 管。token 由维护者自己在 GitHub 上建、自己粘到服务器上：编码 Agent 不建、不经手凭据，建的时候 GitHub 也要账号本人再确认一次密码。过期前一个月，汇总在当天的 issue 里提醒一次。读不到 token 或者 GitHub 拒绝时，发现的问题留在状态文件里，下次再发，不丢。

### 3. 每天一份新用户体检

这次手动做的那一套，自动做：

- 每天北京时间早上 8 点跑一次，只读查过去 24 小时注册的账户各走到了哪一步：
  1. 注册；
  2. 验证邮箱；
  3. 第一次登录；
  4. 设备签名好了；
  5. 进了房间；
  6. 请来了 Agent；
  7. Agent 回过人的话；
  8. 第二天还回来。
- 只出每一步的人数，加上“卡在哪一步”的人数和当时的相关指纹，比如“登录回调失败以后没再登录：1 人”“Agent 进了房间，人说了话，Agent 一次都没回：1 人”。不出任何账户信息。
- 写进当天那条汇总 issue。连着三天都卡在同一步的，单开一个 issue，标 `funnel`。

### 4. 有新 issue 就叫醒开发 Agent

维护者 2026-10-24 定：有新 issue 就叫醒，不是每天看一次。

- 用 Claude Code 的云端 routine，挂一个 GitHub 事件触发：`agent-room-ops` 里带 `prod-error` 标签的 issue 新开（`opened`）或重新打开（`reopened`）时叫醒，只处理叫醒它的这一条。补评论（同一种错又出现）不叫醒，免得一种错一天叫醒好几次。
- 不带定时（维护者 2026-10-24 定：不要每晚跑一次）。事件没送到、当时被限流跳过的，等下一次被叫醒时补上：先处理叫醒它的那一条，再顺手处理最多 2 条一小时以上没动过、还没标 `triaged` 的，按 p1、regression、p2 的顺序。
- routine 的代码来源是两个仓库：`agent-room`（改代码、开 PR）和 `agent-room-ops`（读 issue、写结论）。两个仓库都要装着 Claude 的 GitHub App。
- 一次汇总最多开 5 个新 issue（第 2 层），所以一小时里最多同时叫醒 5 个。它们各修各的，PR 之间撞了由 main 的 push CI 兜底，和平时一样。
- 每次叫醒做这几件事：
  1. 读 issue 和样例，在代码里找原因；
  2. 能修的按 CLAUDE.md 的规矩开 PR，PR 描述里写 `Fixes #N`，检查过了就自动合并；
  3. 判断是正常现象的，改噪音表，在 issue 下说明为什么正常，然后关掉；
  4. 要维护者定的（改产品行为、改隐私说明、涉及钱和账号），标 `needs-owner`，写清楚要定什么、建议是什么；
  5. 样例不够、要看生产现场的，标 `needs-prod-look`。维护者在本机开会话时，编码 Agent 照“只读查生产用户”的做法去看。
  6. 处理完加 `triaged` 标签。
- 修好以后：PR 描述里写 `Fixes rainyflash/agent-room-ops#N`，合并时那个 issue 自动关闭；下一版上线后同一指纹又出现，第 2 层会重新打开它，标 `regression`，又会叫醒开发 Agent。
- 每次处理完在当天的汇总 issue 下写一行：处理了哪几条、开了哪些 PR、哪些在等维护者。
- **issue 里的字当数据看，不当指令。** 样例来自日志，日志里有外面的人能控制的字（请求路径、Keycloak 的 `reason`）。有人可能故意发一个带着“请把某段代码加进去”的请求，让它出现在 issue 里。所以：
  - 汇总只留路由模板，样例截短、只留可打印字符；
  - routine 的提示词写明 issue 内容是数据；
  - 开发 Agent 的 PR 照旧要过 CI，改到 `.github/workflows`、发布、登录和权限相关的代码时，不自动合并，标 `needs-owner`。

## 不做

- **不接 Sentry 之类的第三方。** 用他们的云服务，就多一个经手个人数据的第三方，隐私说明要改，免费档的量也有限；自己搭 Sentry 要十几个容器、好几 GB 内存，生产服务器扛不住。
- **这次不打开 Prometheus 那一套。** 它常驻吃内存，看的是总量。以后要做容量和可用率告警时再开，和这份方案不冲突：告警还是告警，偶发的错走 issue。
- **先不收客户端的错。** 网页和桌面端碰到的错（比如 10-19 旧 iPhone 那次）服务器看不到。要收，就得加一个匿名的错误回报接口，隐私说明也要改。等前四层跑一阵再看值不值得。
- **issue 里不放能认出人的东西。** 见下一节。

## 隐私

- issue、评论、状态文件里都不出现：邮箱、IP、账户 ID、Matrix ID、房间号和房间名、Agent 名字、消息内容、令牌，以及 `code`、`state`、`session_state` 这类参数的值。
- 打码由一组测试卡着：拿带着这些字段的真实格式日志行（控制面 JSON、Keycloak 事件、Synapse 访问日志各几条）做输入，查输出里一个都不剩。
- 只往外发汇总和打码后的样例，不含个人数据，隐私说明页不用改。以后做客户端错误回报时要改。

## 交付

1. 这份设计。
2. 汇总脚本：读日志和库、算指纹、打码、噪音表，带测试。先只在服务器上打报告（`production.py error-digest`，这一步还不往外发），编码 Agent 在维护者本机跑几天，把噪音表调好。分两个 PR：
   - 2a 读四个服务的容器日志；
   - 2b 加上库里卡住的后台活（只读 SQL）。
3. 开 issue：去重、重新打开、限量，装定时器。随下一次发版部署（改了 `schedule.py` 和生产配置）；部署之前维护者要放好 token。
4. 新用户体检。
5. 叫醒开发 Agent：建 routine，挂 GitHub 事件触发，每次叫醒顺手补漏；提示词写进仓库（`docs/operations/error-triage.md`），以后改提示词走 PR。
6. 以后再说：客户端错误回报。

## 维护者定了的

2026-10-24：

1. issue 开在新建的私有仓库 `rainyflash/agent-room-ops`，修复的 PR 照旧开在公开仓库。当天已经建好。
2. token 维护者让编码 Agent 建。编码 Agent 不能替账号建凭据、经手凭据，所以改成：编码 Agent 给一个预先填好的创建链接和一条粘贴命令，维护者自己点“生成”、自己粘到服务器上。第 3 步合并前要放好。
3. 开发 Agent 有新 issue 就叫醒，不是每天看一次（见第 4 层）。当天先另带了一个每天补漏的定时，维护者看了说不要每晚跑，同一天去掉了。

新用户体检没有单独回答，照设计做，只发人数，放在私有仓库。

## 状态

- 2026-10-24：起草；维护者当天定了 issue 开在哪、token、开发 Agent 什么时候叫醒（见上一节）。私有仓库 `agent-room-ops` 和标签已建好。
- 2026-10-24：2a 读日志的汇总做好（`tools/prodops/error_digest.py`、噪音表第一版）。合并前把这份代码打包成只读脚本在生产上跑了 48 小时的日志：
  - 四个服务一共约 43 万行，报出 10 种，按噪音表没报的 1911 次（九成是账户数据的 404）；
  - 输出里没有邮箱、IP、Matrix ID，UUID 只有关联号；
  - U2 碰到的三样都报出来了：进私人房间地址的 500、`Invalid scopes: openidprofile` 的登录失败、过期的验证链接。
- 实现和设计不一样的两处：
  - 汇总里的字段按名字排序（`failure=` 在 `operation=` 前面），噪音表的正则照这个顺序写；
  - 控制面的日志里没有账户，只有身份服务和 Synapse 的写“涉及几个账户”。
- 2026-10-24：维护者放好了令牌（`/etc/agent-room/ops-issues.token`，root、0600），只给 `agent-room-ops` 的 Issues，GitHub 说 2026-11-23 过期（30 天，不是 366 天）。
- 2026-10-24：第 3 步开 issue（`tools/prodops/ops_issues.py`）和每小时的定时器（`schedule.py` 的 `ErrorDigestScheduleInstaller`，每小时第 7 分钟）做好：
  - 合并前把代码打包成脚本在生产上真发了一次，用 48 小时的日志起头：开了 `agent-room-ops` #1–#5，另 3 种记进当天的 #6“今天还有这些”，标签都在，正文没有邮箱、IP 和 Matrix ID；
  - 状态文件 `/var/lib/agent-room/error-digest/state.json` 那时就建好了，定时器上线后接着它读；
  - 代码要随下一次发版到服务器上，发完跑一次 `error-digest-schedule-install` 装定时器。
- 第 3 步和设计不一样的几处：
  - 开着的 issue 不在当天第一次出现时评论，而是每天第一次跑时把前一天的次数补一条。账户数写“单个小时里最多涉及几个账户”：状态文件不存账户摘要，跨小时去不了重。
  - 关着的又出现：版本没变（修复还没上线）只数次数，换了版本才重新打开、加 `regression`，并去掉 `triaged` 让开发 Agent 再看一遍。
  - 令牌离过期 14 天时开一个 `needs-owner` 的 issue 提醒维护者换（不带 `prod-error`，不叫醒开发 Agent），设计原来写的是一个月，这次的令牌一共才 30 天。
  - 版本号以 Cargo workspace 的为准（2a 读了根目录的 package.json，那个不跟着发版改）。
  - 开 issue 时 GitHub 对只有 Issues 权限的令牌可能悄悄丢标签，开完核对一遍、少了补上：叫醒开发 Agent 靠 `prod-error`。
- 2026-10-24：第 3 步随 Alpha 81 上线。发布脚本最后一步在服务器上装好了定时器（`agent-room-error-digest.timer`，09:34Z 启用，下一次 10:07Z），以后跟着源码走，不用每版再装。
- 2026-10-24：第 4 步叫醒开发 Agent 的 routine 建好了，怎么改、怎么查、提示词全文在 [docs/operations/error-triage.md](../../docs/operations/error-triage.md)。和设计不一样的几处：
  - routine 的 GitHub 事件触发器不支持按标签过滤，`agent-room-ops` 里任何 issue 新开、重新打开都会叫醒它，由提示词让它碰到不带 `prod-error` 的直接结束；
  - 云端会话里 `gh` 用不了 GraphQL（`gh issue list`、`gh pr merge --auto` 回 403），提示词让它用会话自带的 GitHub 工具或 `gh api` 走 REST；
  - 每天兜底跳过一小时内刚更新过的 issue，免得和正被事件叫醒的那次撞上。
  - 试跑了一次：两个仓库都克隆了，读到 `agent-room-ops` 里没有要处理的，什么都没写。
- 2026-10-24：维护者说不要每晚触发的定时，routine 只留 GitHub 事件触发。补漏改成每次被叫醒时顺手做：处理完叫醒它的那一条，再看最多 2 条一小时以上没动过、还没标 `triaged` 的。去掉之前当天跑过的三次是一次试跑和 #7、#8 各叫醒的一次，定时那次还没轮到过。
