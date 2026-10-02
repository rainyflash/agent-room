# 给编码 Agent 的工作说明

Agent Room 的日常开发交给编码 Agent 做。2026-09-24 以前在维护者本机的 Claude Code 里进行，之后转到云端 Agent。这份文件记下那段时间积累的约定、坑和当前进度。环境搭建、架构规则和测试要求见 [CONTRIBUTING.md](./CONTRIBUTING.md)，这里不重复。

## 授权与节奏

- 维护者把项目全权交给你：常规维护自己做完，不必每一步回来确认。检查通过后自己 squash 合并自己开的 PR。
- 仍要先问维护者的只有三类：
  - 不可逆、又没授权过的破坏性操作；
  - 只有账号所有者能做的事，比如批准 Keycloak 设备码、登录宿主 CLI；
  - 仓库设置与分支保护。
- main 关掉了“合并前必须与 main 同步”：PR 自己的 CI 绿了就能合，PR 之间的冲突由 main 的 push CI 兜底。
- 发布期间 main 不冻结，但不能合并改动 `.github/workflows` 的 PR。原因：没有 workflow 权限的令牌，不能在工作流与 main 不同的提交上创建 Release 或标签。
- 攒够一批修复再发版。版本号提升可以和最后一个修复放在同一个 PR。
- 不用付费的大规格 Runner，`tools/check-actions-pinned.mjs` 会拦。
- 不做公开测试门禁（72 小时常驻、额外服务器、外部评审之类）。维护者说这些做不了，也没必要。优先做他直接体验到的东西：界面、流程、性能、他报的问题。发布本身的严谨（CI、签名、实机验收）照旧。

## 写代码与提交

- 文档、提交说明和 PR 描述用中文，写成大白话。用户看得到的文案中英文一起改。
- 提交标题用 Conventional Commit，比如 `feat(network-agents): …`；squash 合并后就是 main 上的一条提交。
- 大功能先写设计文档，再按文档分步交付，每步一个 PR。
  - 实际做法与设计不一样时，在同一个 PR 里改设计文档，并在它的“状态”一节记一笔。
  - 依赖还没合并的上一步时开叠加 PR（base 设为上一步的分支）。上一步合并后，`gh pr edit <号> --base main`，再 `git rebase --onto origin/main <旧基提交>` 并强推。
- 推之前在本地检查：
  - `cargo fmt --all -- --check`；
  - 与 CI 同款的 clippy：`cargo clippy -p agent-room-application -p agent-room-content-adapter -p agent-room-postgres-adapter -p agent-room-control-plane -p agent-room-bridge --all-targets -- -D warnings`；
  - 改了 Markdown 或前端代码就跑 prettier（`specs/` 在 `.prettierignore` 里）。
- clippy 开了 `too_many_lines`（100 行），函数太长就拆出辅助函数。
- 改了 `Cargo.lock` 或 `pnpm-lock.yaml`：在同一个 PR 里提交 `python tools/license_inventory.py generate` 的结果，否则 PR 的“格式、类型与测试”会红。
- 新增 MCP 工具时，同步改三处，否则 Python 工具测试和发版的 MCP 门禁（`tools/mcp_release_gate.py`）会挂：
  - `tools/mcp_release_gate.py` 的 `EXPECTED_TOOL_ANNOTATIONS`；
  - `tools/mcp_client.py` 的工具集合与 schema 校验；
  - `apps/agent-room-mcp/src/agent_room/server.rs` 里列出全部工具的测试。
- 网络 Agent 远程 MCP 的服务说明最多 1536 字节，有测试卡着。
- CLI 与 MCP 的测试，要在设了和没设 `CLAUDE_CODE_SESSION_ID` 两种环境下都能过（`env -u CLAUDE_CODE_SESSION_ID cargo test …`）。

## CI

- main 有 8 个必需检查：
  - 格式、类型与测试；
  - Web 真实浏览器验收；
  - Windows 客户端运行时原生检查；
  - 3 个 CodeQL；
  - 供应链与物料清单；
  - PostgreSQL、Matrix、对象存储与协议集成。
- 最后两个只在 `gh workflow run ci.yml --ref <分支> -f suite=all` 派发时才真跑。在 pull_request 里它们是 skipped，也算满足。
- 真实数据库、真实 Synapse 的测试标了 `#[ignore]`，只在派发的集成作业里跑。例如控制面的 `real_dependency_tests::` 和 matrix-adapter 的 `real_synapse`。
- 改到这些地方时，在 PR 分支上派发一次 `suite=all` 拿证据。派发里的红不挡合并，但会挡下一次发布（发布调度跑的也是 `suite=all`），所以要单独跟进。
- 改 `apps/desktop/src-tauri/windows/` 下的安装器钩子时，PR 上会跑“Windows 安装器钩子”工作流（`tools/windows_installer_hooks.py`）：编译精简安装器，实跑运行中覆盖安装与卸载，一分钟左右。
  - 它按路径触发，不是必需检查，红了同样不能合。
  - 本机跑要加 `--isolated`；不加会拒绝运行，免得结束正在用的 Agent Room。
  - 升级 `@tauri-apps/cli` 时，同步更新工具里固定的模板提交和哈希，单元测试会提醒。
- 已知的偶发失败，重跑即过：
  - “真实网页登录与会话恢复”偶发 `null pointer passed to rust`。这是 matrix-js-sdk 退出登录时 rust-crypto 备份检查的竞态。
  - 同一个用例偶发 `Failed to process outgoing request 0: AbortError: signal is aborted without reason`：退出登录时 `stopClient` 中止了还在发的加密请求，同一类竞态。只重跑失败的作业（`gh run rerun <run> --failed`）即可。
  - 无头验收里 Synapse 偶尔没起来。
- 上游出了新的安全公告、供应链作业变红时：
  1. 先试升级依赖；
  2. 升不了，就按 `deny.toml` 里已有例外的写法加一条带理由的精确例外；
  3. 再开一个 `security-dependency` issue 跟踪。
- CI 里的 Web 单元测试跑的是根目录的 vitest 配置，不加载 `apps/web` 的 setup。跨平台断言要在用例里写明访客系统。

## 代码里的坑

- **SQLite 只能链接一份。** matrix-sdk-sqlite、rusqlite（`crates/matrix-adapter/src/store_recovery.rs`）和 Bridge 本地存储用的 sqlx-sqlite 共用 `libsqlite3-sys`。
  - 升级 matrix-sdk 或 rusqlite 之前，先查 sqlx-sqlite 允许的 `libsqlite3-sys` 版本范围。
  - matrix-sdk 0.19 目前就卡在这里，见 issue #101。
- **加密房间首次见到即信任**（[ADR 0009](./docs/adr/0009-encryption-trust-on-first-use.md)）。只信任由主人签名的设备，不要求逐个核对安全码；维护者明确否决过“发送前强制核对”，别加回去。有两处刷新不能去掉（#116），去掉后 Agent 之间会收不到消息：
  - 发送前刷新成员身份；
  - 收到未签名或未知设备的消息时，刷新后再判一次。
- **Synapse 的相同状态去重。** 与当前状态完全相同的状态事件，Synapse 直接返回旧事件 ID，也不做权限检查。所以测“撤权后被拒”要换一份内容。
- **Synapse 会把一模一样的同步请求缓存两分钟**（`sync_response_cache_duration`，键是用户、设备、超时、起点、过滤器、`full_state` 等）。两分钟内再发一次不带起点的同步，拿到的是上一次的结果。2026-10-02 网络 Agent 凭口令进私人房间后马上发言，被说成不在房间里，就是因为加密客户端拿到的是加入之前的缓存。现在网关每次不带起点的同步都换一个超时值（`network_gateway/encrypted.rs` 的 `initial_sync_request`）；别的地方要反复做不带起点的同步，也得这样。
- **Synapse 没接 MAS 时，已有签名身份的账户换签名身份一律要交互认证。** 管理接口 `_allow_cross_signing_replacement_without_uia` 只在接了 MAS 时起作用，只有应用服务的请求例外（MSC4190）。所以人的设备自动签名重建签名身份时，新签名公钥由控制面以应用服务身份冒充本人上传；应用服务注册为此有一个覆盖所有本地用户的非独占命名空间（ADR 0011 的“修订”）。
- **matrix-js-sdk 的 `bootstrapCrossSigning` 看到本机有签名私钥就不上传公钥。** 上次上传被打断（页面跳走）时，本机留着私钥、服务器上却没有签名身份，它也照样跳过。账户还没有签名身份时要用 `resetEncryption` 从头建，别用 `bootstrapCrossSigning`。
- **Windows 具名管道会踩坏堆。** tokio 的客户端在“丢弃连接”与“I/O 驱动处理同一管道”并发时会出这个问题（上游 mio#2011）。#145 起，本地客户端连接都放在专用的单线程运行时线程上跑；上游修好之前别拆。
- **Windows 凭据管理器会吞掉重叠的写入和删除。** 产品代码经 `SystemCredentialStore` 逐个调用，新代码别直接用 `keyring`。
- **生产对象备份用 `rclone/rclone`**（#271，Alpha 59 起）。MinIO 把开源项目归档了，`minio/mc` 的镜像和程序都已下架；Alpha 58 时临时重建的同名镜像和 `/root/mc-rebuild` 已在 Alpha 59 上线后删掉。`object-backup` 这类按需运行的容器平时没有容器在用，`docker image prune -a` 会把它们的镜像一起删掉，清镜像时要排除。
- **真实 Synapse 测试里的加密房间。** 参与者要用全新的受管账户：种子账户每次登录都会得到一台缺私钥的新设备。

## 产品决定（已定，别再问）

- 核心体验是“一个按钮把 Agent 请进来就能聊”，参照桌面端的“接入 Agent”对话框，流程要一步到位。
- Agent Room 是通用 Agent 软件，不做只服务某个 Agent 应用的功能、界面或提示词；后台回复按宿主恢复任务属功能需要，保留。
  - 接入只有网络、MCP、命令行三种通用方式，MCP 只给一份通用 JSON。
  - 给 Claude Code 装技能、按应用一键写 MCP 配置、Codex 插件发行包都已去掉，别加回来。
- 自动回复授权的默认有效期是 30 天（已实现）。
- 只凭网络接入的 Agent（[ADR 0010](./docs/adr/0010-network-agents.md)）有两条维护者已接受的取舍：
  - 公开大厅允许没有账号的网络 Agent；
  - 网络 Agent 凭口令进的私人房间，服务器能读到发给它的消息。
- Mac 版已经过苹果公证。别再在文档里教用户去“隐私与安全性”里放行。
- 人的设备自动签名，不要恢复密钥（[ADR 0011](./docs/adr/0011-automatic-device-signing.md)，维护者 2026-10-02 定）：服务器保管账户签名用的钥匙，任何设备登录就自动签名、自动找回加密历史；界面上不再有恢复密钥、恢复口令和设备核对，别加回来。代价是部署方能替账户签设备，维护者接受。设计在 [specs/device-signing/design.md](./specs/device-signing/design.md)。

## 当前进度（2026-10-02）

### 只凭网络接入的 Agent

- 设计在 [specs/network-agents/design.md](./specs/network-agents/design.md)，实现以它为准，进度记在它的“状态”一节。
- 第 1、2、3 步都已完成。
  - 第 3 步：网络 Agent 凭口令进端到端加密的私人房间，服务器替它管加密存储。
  - 合并记录：3a #170、3b #171、3c #172、3d #173、3e #174、3f 提示 #176、3f 验收 #178。
  - 3c 和 3d 必须在同一个版本里发布，现在都在 main 上。
- `tools/headless_acceptance.py` 有一轮私人房间的真实验收：凭口令进私人房间，与本机 Agent 加密收发，控制面重启后照常，删掉网络 Agent 的存储后重建照常。只在派发 `suite=all` 时跑。
- 这轮验收失败时，会在作业输出最后打出两段汇总，CI 日志只能看到最后 5000 行，先看这两段：
  - 控制面告警与长轮询调试的去重汇总；
  - 发送方 Bridge 的密钥分享日志。
  - Bridge 文件日志的过滤规则可以用 `AGENT_ROOM_BRIDGE_LOG_FILTER` 换掉。
- 本机 Bridge 的加密身份冲突恢复也改成换一台新设备（`POST /agent-instances/{id}/matrix-device`），和 3e 一样避开 Synapse 留着旧交叉签名的问题。旧的 `/matrix-session`（同一设备重签）只为旧版 Bridge 保留。

### 加密房间里的消息解不开

- 2026-09-28 维护者房间里 329 条消息全部解不开。根因：matrix-js-sdk 42.2.0 配 crypto-wasm 18.5.0 时，每收一批 to-device 消息就多传 50 个一次性密钥；积压超过客户端保留的 5000 个后，Synapse 按上传先后发给 Agent 的是早被丢掉的密钥，Olm 通道建不起来。
- 已做：升级到 42.4.0 #201（带真实 WASM 的回归测试）；解不开的消息汇总成提示 #202；维护者同意后清掉了他两台设备在服务器上积压的密钥（备份在服务器 `/root/maintenance/`）。
- 建坏的通道和已经错过的房间密钥不会自己好。找回历史的设计在 [specs/room-key-recovery/design.md](./specs/room-key-recovery/design.md)：人的设备发现缺密钥就请 Agent 经新的 Olm 通道重发它自己的房间密钥。
  - 设计 #204、协议 #205、Agent 这边 #206（真实 Synapse 集成测试通过）、人这边 #208、“先验证设备”这条路 #211 都已随 Alpha 55 发布。
  - 第 4 步真实浏览器验收还没做。维护者房间在 Alpha 55 上能不能找回，就是第一次真机验证。
  - 看重发结果：Alpha 55 的 Bridge 文件日志默认只记 `agent_room_bridge`，看不到。#217 起默认也记 `agent_room_matrix_adapter::room_keys=debug`：成功是 info“按请求重发了这台设备的房间密钥”，没重发是 debug“没有重发房间密钥”并带原因。在那之前只能看房间里的提示是否消失。
  - 第二期：新请进房间的 Agent 也要读到加入前的消息，其他 Agent 的和人的都要（维护者 2026-09-29 决定）。设计在 [specs/room-key-recovery/pre-join-history.md](./specs/room-key-recovery/pre-join-history.md)。
    - Agent 这边分 2a（matrix-adapter 请求与导入）和 2b（Bridge 给解不开的事件预留位置，找回后写回原位）。人这边（第 3 步）是网页端的 `MatrixRoomKeyResponder`：只重发这台设备建的会话，只回答此刻在房间里、带在线状态事件、由主人签名的 Agent 设备。
    - 功能上线前隔离的事件没有预留位置，找不回来。
    - 随 Alpha 56 发布。第一次真机验证要等维护者往有历史的私人房间请一个新 Agent：Bridge 文件日志有 info 级“找回房间密钥后重读了之前解不开的消息”。
  - 网页端设备（“Agent Room Web”）没由主人签名时，Agent 按规则扣下房间密钥。#211 起这台设备的找回请求先扣着，签名同步到本地后自动发出；发送方因设备没签名而拒绝分发的消息也会请求重发。设备自动签名第 4 步起，提示里不再有“验证这台设备”按钮，只说正在自动签名。

### 人的设备自动签名

- 2026-10-02 维护者定了：服务器保管账户签名用的钥匙，任何设备登录就自动签名（[ADR 0011](./docs/adr/0011-automatic-device-signing.md)），设计在 [specs/device-signing/design.md](./specs/device-signing/design.md)，进度记在它的“状态”一节。
- 已合并：设计 #289；第 2 步控制面保管钥匙 #290（表 `principal_encryption_key`、`/account/encryption-key`、`/account/encryption-reset`）；第 3 步设备上的自动签名 #291（`ensureDeviceSigned`，进度在 `DeviceSigningStatus`）。
- 第 4 步 #293 去掉界面上的恢复密钥和核对：安全页只说这台设备“已就绪 / 正在准备 / 出错可重试”，设置上的提醒点只在出错时亮。浏览器验收用 `security-center.html`，加 `?signing=failed` 看出错和重试。
- 重建签名身份改由应用服务代传新签名公钥（修正第 2、3 步，见上面“代码里的坑”）：网页端在 fetch 外包一层（`cross-signing-upload-route.ts`），把带我们认证标记的上传改送 `POST /account/encryption-reset`。发版部署时候选改了 Synapse 配置会先重启一次 Synapse（`release_deploy.py`）。
- 第 5 步 #294：Agent 发现主人核对过又换了签名身份时，自动撤掉旧的核对、记住新身份（matrix-adapter 的 `AgentOwner`，Bridge 恢复连接时交给它）。不然以前跟主人核对过安全码的 Agent 会整个房间发不出消息。第 2 到第 5 步要在同一个版本里发。
- 维护者的账户没有密钥存储、没有备份，升级后第一台打开的设备会重建一次签名身份。

### 界面翻新

- 2026-09-29 维护者要求“全部翻新”：流程减到必要的几步，全站一套组件，说人话。设计在 [specs/interface-renewal/design.md](./specs/interface-renewal/design.md)，进度记在它的“状态”一节。2026-09-30 七步全部做完（设计 #229，实现 #230–#246），已随 Alpha 57 发布。
- 视觉仍按 [游戏大厅界面重做](./specs/game-lobby-refresh/design.md) 的令牌与形状规则；那次只换了外观，这次改流程、文案和组件。
- 以后加界面：先在 `packages/ui-system` 和 `shared/ui` 里找现成组件；用户看得到的说法按设计文档的“用词”一节；排查才看的 ID、错误码收进“详情”。新文案中英文一起加，删功能时顺手删掉只有它用的文案和样式。
- 第 2 步（一屏的“接入 Agent”对话框）和第 3 步（新的“我的 Agent”、这台电脑、去掉浮动面板和 `/onboarding`）要在同一个版本里发布。
- `/onboarding` 已去掉：桌面端没有默认 Agent 时连接服务照样运行，MCP 和命令行的 Agent 都走本机会话，不依赖默认 Agent。
- 浏览器验收里桌面端的“我的 Agent”页用 `e2e/fixtures/my-agents.html`（加 `?browser` 是网页端），`window.__agentRoomFixtureControls.arriveAgent()` 模拟一个 Agent 接走接入对话框挂着的人物。
- 设置页在 `/settings/<分节>`（通用 / 安全 / 这台电脑 / 关于）；浏览器验收用 `my-agents.html?settings=<分节>`，安全一节用 `security-center.html`。新的浮层提示一律放进根布局的提示栈（`ToastStack` + `Toast`），别再单独 `position: fixed`。
- 没进房间时的页面（连接、登录没完成、进大厅、房间打开中或打不开、找不到页面、配置出错）共用 `shared/ui/entry-shell.tsx` 的 `EntryShell` + `EntryCard`：一张居中卡片，错误码和地址收进“详情”，出错给“回到房间”。
- 登录页的标志由 `tools/sync-brand-assets.mjs` 从网页端同步；`tools/tests/test_brand_consistency.py` 卡着登录主题的颜色令牌和标志不和网页端走偏。
- “新建房间”“换个房间”只有 `RoomActions`（`features/room-directory`）一个入口组件，房间菜单、“房间”页和夹具都用它。浏览器验收的 `lobby-scene.html?features=1` 里有两个待答复的邀请（Research lab、Budget review），加入和拒绝都能真走一遍。
- 房间设置是房间页上的一个对话框（`RoomSettingsDialog`），不在房间菜单的抽屉里。`lobby-scene.html?private` 把夹具房间当成你是房主的私人房间，四节都能看到；浏览器验收用 `openRoomSettings(page, '节名')` 打开。

### Agent 怎么看房间里的消息

- 2026-09-30 维护者要求考量 Agent 自身的体验。调研结论和改法在 [specs/agent-reading/design.md](./specs/agent-reading/design.md)，按文档分步交付，进度记在它的“状态”一节。
- 方向：三种接入用同一套模型，“新消息收件箱 + 按需查看”（按 ID 取、看前后、往前翻、只看提到我的）。维护者定了：网络接入每个房间留最近 500 条；1000 字以内给全文，更长的只给开头、要全文按 ID 取。
- 等消息是维护者最看重的，设计在 [specs/agent-reading/waiting.md](./specs/agent-reading/waiting.md)，排在查看接口之前做。维护者定了：
  - 默认跟它有关的才叫醒（人说的话都算，点了别人的除外；Agent 说的要点它或回复它；主人说话总能叫醒）；
  - 来了以后多等一会再交（防抖），或者等指定的几个人都回了；
  - 定时看一眼能设，默认不开；
  - 后台回复：主人，加上私人房间里点名或回复它的人。
- 等消息的进度：规则 #257（`bridge-ipc` 的 `wake`）、网络接入 #258、本机 3a #259 已合并。3a 规则放在 agent-client 的 `InboxWaiter`，MCP、CLI 都用它，Bridge 只认一个 `keepWaiting`；3b 再把“每秒问一次”换成 Bridge 挂起等待。3b（Bridge 挂着等，`waitMs`）#262。本机的“主人说话总能叫醒”：Bridge 授权和刷新设备时记下主人，存成数据目录下的 `owner.json`，`GetSelf` 带 `owner`，MCP 和命令行等消息时问一次。
- 打字时再等等（第 5 步）：5a #264 网页端发“正在输入”、规则多了“此刻谁在打字”、网络接入跟上；5b 本机 Bridge 跟上：同步时记下（`TypingWatch`），`WaitInbox` 的回应带 `typing`，打字的人变了挂着等的请求马上返回。叫醒它的人在打字也算对话没停：还在打就接着等，停下以后再等防抖的 5 秒（网页端点发送时先说停了、话随后才到），最多 30 秒。“正在输入”的记录（`bridge_ipc::typing::TypingRooms`）网关和 Bridge 共用。
- 无头验收有一轮等消息的规则（`verify_waiting_rules`，第 6 步）：没点名不叫醒、连发三条叫醒一次、等齐两个人、定时看一眼（要等满 1 分钟）、本机 MCP 按同样的规则等。只在派发 `suite=all` 时跑。
- 后台回复（第 4 步）：接收器也用 `InboxWaiter`，叫醒判断是 `ReceptionPolicy::wakes`（主人说的、跟它有关的话；私人房间里点名或回复它的人；别的 Agent 叫不醒）。防抖后整批交给宿主一次，宿主回空正文就记成 `no_reply`。服务器上的进度还是一组“锚点（交出去的最后一条）、回复目标、提交 ID”。改宿主提示要在真机上跑一次 `agent-room receiver doctor`，Codex 和 Claude Code 都要。
- 测本机等消息别用“只给一次”的假 Bridge：交之前会重读一遍，假的得像消息库一样按游标给；暂停时间的测试里假的给不出来就会一秒一秒空转，内存一路涨。
- 一次 @ 更多人和 @所有人：设计在 [specs/agent-reading/mentions.md](./specs/agent-reading/mentions.md)。维护者 2026-10-01 定了：@ 人数不设上限（硬上限 200，另卡总字节）；@所有人只在私人房间里用，人和 Agent 都能用。@所有人放在 `preview.mentionsEveryone`，不放进 `conversation`（旧版网页对它严格校验，多一个字段整条消息会消失）；接收方只认加密消息上的这个开关。
  - 第 2 步（上限 200、加起来 12 KB）#275；第 3 步（@所有人）#276；第 4 步无头验收一轮（`verify_mentions_everyone`、`verify_lobby_refuses_everyone`），只在派发 `suite=all` 时跑。上限的常量在领域层（`MAX_CONVERSATION_MENTIONS`、`MAX_CONVERSATION_MENTION_BYTES`），IPC、MCP、网络接口都引用它，别再各写各的数字。
  - “只认加密消息上的”放在 Bridge 和网关共用的解析里（`parse_preview` 拿事件的 `end_to_end_encrypted()`），之后的存储、读消息、等消息都不用再管加密。
  - 后台回复只认人：Agent 发的 @所有人 和 Agent 点名一样叫不醒开着后台回复的 Agent。
- 按需查看（第 3 步）：3a #283 是 Bridge 与 IPC 4.3（`GetMessages`、`MessagesAround`、`RoomHistory`），3b #284 是 MCP 的 `agent_room_get_messages`、`agent_room_room_messages` 和命令行的 `show`、`around`、`history`。从 3b 起本机收件箱的长消息只给开头；后台回复交给宿主之前按 ID 取回全文（`agent-reception` 的 `with_full_text`），宿主照旧读到整条。
  - 本机 MCP 的服务说明（`SERVER_INSTRUCTIONS`）已经 1522 字节，测试卡在 1536 以内，要加话得先删别的。
- 接入说明有两个地址：`/agents.md` 和 `/agents.txt`，同一份内容，都按 `text/plain` 发。接入对话框给 `.txt`：有的网页读取器按网址结尾猜类型、不看响应头，见了 `.md` 就当 markdown 拒收（维护者 2026-10-02 遇到过）。
- 只会浏览网页、发不了请求的聊天助手只能靠它所在的应用加 MCP 连接器（`{API}/mcp`）接入，有的应用要付费版、有的根本没有；别再想“把加入和发言做成能直接打开的链接”，令牌会进 URL（#251 的 PR 描述里有完整取舍）。

### 版本与其他

- Alpha 61 正在发布：人的设备自动签名（设计 #289，实现 #290、#291、#293、#294、#295），界面上不再有恢复密钥和设备核对。#295 改了 `tools/prodops/render.py`（应用服务注册），实机验收要一次设备码；部署时 Synapse 会重启一次。
- Alpha 60 已于 2026-10-02 公开，见 [发布记录](./specs/agent-access/alpha60-release.md)。
  - 网络 Agent 凭口令进私人房间后马上就能发言（#285，Synapse 同步缓存）、进房间后成员栏马上看得到它（#286）；签过名的设备不再每次报“需要签名”（#287）；Agent 按需查看消息（#283、#284）；接入说明多一个 `/agents.txt`（#282）。
  - #282 改了 `tools/prodops/render.py`（Caddy 配置），发布门禁把这个文件算作登录相关代码，实机验收要了一次设备码。长期验收设备换成 Alpha 60 那台（`agent-room.alpha60.acceptance.fresh-device`）。以后改 Caddy 配置，发版前先跟维护者约好批准设备码的时间：码 10 分钟就过期。
  - 版本 PR #288 合并到网页上线约 1 小时 15 分钟，其中约 13 分钟在等设备码。
- Alpha 59 已于 2026-10-02 公开，见 [发布记录](./specs/agent-access/alpha59-release.md)。
  - 消息带房间名、标出进房间之前的（#272、#274）；一条消息最多点名 200 人（#275）；私人房间里 @所有人（#276，验收 #277）。
  - 对象备份改用 rclone（#271），生产上已经跑通；Alpha 58 时临时重建的 `minio/mc` 镜像和 `/root/mc-rebuild` 已删（维护者同意）。
  - 发布提交上的完整 CI 被新公告 RUSTSEC-2026-0318 挡住：我们的设备间消息都用 `AllDevices`，不受影响；修复要 Matrix SDK 0.19，同样卡在 #101。`deny.toml` 加例外 #280（跟踪 #279），#280 的合并提交成为发布提交，候选重建一次（第一次的草稿改名让位）。
  - 没改登录代码，复用长期验收设备，不要设备码。#278 合并到网页上线约 1 小时 55 分钟，其中约 1 小时花在公告和重建上。
- Alpha 58 已于 2026-10-01 公开，见 [发布记录](./specs/agent-access/alpha58-release.md)。
  - Agent 等消息的规则（设计 #255，实现 #257–#267），以及 Agent 读到的每条消息标出是不是自己发的、有没有提到它、回复的是哪句 #253。
  - #263 改了 Bridge 授权，实机验收走了一次新设备授权，维护者批准了设备码；长期验收设备换成 Alpha 58 那台（`agent-room.alpha58.acceptance.fresh-device`）。版本 PR 合并到网页上线约 2 小时 16 分钟。
  - Mac 公证第一次失败：苹果要开发者团队先同意新的协议（HTTP 403 “A required agreement is missing or has expired”），只有维护者能在苹果开发者网站点同意。同意后 `gh run rerun <候选> --failed` 即可（草稿发行还没建时可以安全重跑），重跑后要重新挂 `approve_pending.py`。
  - 部署前预检要求可用磁盘不少于 20 GiB。备份仓库占约 24 GB（最近 8 小时每 15 分钟一份全量基础备份，每份约 600 MB），这次靠清掉旧版镜像腾出空间。
- Alpha 57 已于 2026-09-30 公开，见 [发布记录](./specs/agent-access/alpha57-release.md)。
  - 界面全部翻新（设计 #229，实现 #230–#246），另含房间密钥日志 #228 和依赖安全升级 #231、#234。
  - 6c #245 改了登录主题，实机验收走了一次新设备授权，维护者批准了设备码；长期验收设备换成 Alpha 57 那台（`agent-room.alpha57.acceptance.fresh-device`）。版本 PR 合并到网页上线约 65 分钟。
  - 真实登录验收只在派发时跑：翻新时改了页面，它在 6c 分支的派发里才红（4b 挪走了 Matrix ID），修复随版本 PR #247 合并。以后改界面，发版前记得派发一次 `suite=session`。
- Alpha 56 已于 2026-09-29 公开，见 [发布记录](./specs/agent-access/alpha56-release.md)。
  - 包含新请进房间的 Agent 读到加入前的消息 #222–#225（Agent 的和人的都行）、Agent 连不上 Bridge 时在后台拉起桌面端 #221、退出桌面端时 Bridge 有序退出 #220、默认文件日志记下房间密钥重发的结果 #217。
  - 登录相关代码没变，实机验收复用 Alpha 53 那台长期验收设备，没要设备码；版本 PR 合并到网页上线约 57 分钟。
  - 升版本时主检出的 `node_modules` 还是 #215 之前装的，`bump_release_version.py` 刷新许可证清单报缺包；`corepack pnpm@10.28.0 install --frozen-lockfile --force` 后重跑 `python tools/license_inventory.py generate` 即可。
- Alpha 55（2026-09-29）见 [发布记录](./specs/agent-access/alpha55-release.md)：找回解不开的历史 #206/#208/#211、私人房间一键生成网络 Agent 口令 #207 等。
- Alpha 54（2026-09-28）见 [发布记录](./specs/agent-access/alpha54-release.md)：一次性密钥积压修复 #201、解不开的消息提示 #202 等。
- Alpha 53（2026-09-28）见 [发布记录](./specs/agent-access/alpha53-release.md)；它那次新设备授权的设备一直用到 Alpha 56。
- 网络 Agent 三步都已上线，生产总开关开着。
- 桌面“第三层接入”（桌面直接拉起 Agent 会话，不用开终端）以后单独设计。
- `archive/codex/*` 是 2026-09-05 Codex 时期没合并的实验分支（游戏化大厅、显式 MCP 任务会话等），从维护者本机备份上来，只作存档，不要合并。

## 发布

- 流程见 [docs/release-workflow.md](./docs/release-workflow.md) 与 [docs/operations/signed-releases.md](./docs/operations/signed-releases.md)。公开发行在受保护的 `release/<标签>` 分支上锁定运行。
- 实机验收（`tools/release_qa.py`）和生产部署只能在维护者的 Windows 机器上跑，因为需要：
  - 装好的桌面端；
  - 到生产服务器的 SSH；
  - `artifacts/releases/` 下的私有输入（不入库）；
  - 维护者亲自批准的设备码（绝不能替他批准）。
- 云端 Agent 只改代码、合并 PR，发版由维护者在本机发起。
