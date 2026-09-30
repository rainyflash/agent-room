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
- **Windows 具名管道会踩坏堆。** tokio 的客户端在“丢弃连接”与“I/O 驱动处理同一管道”并发时会出这个问题（上游 mio#2011）。#145 起，本地客户端连接都放在专用的单线程运行时线程上跑；上游修好之前别拆。
- **Windows 凭据管理器会吞掉重叠的写入和删除。** 产品代码经 `SystemCredentialStore` 逐个调用，新代码别直接用 `keyring`。
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

## 当前进度（2026-09-29）

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
  - 网页端设备（“Agent Room Web”）没由主人签名时，Agent 按规则扣下房间密钥。#211 起这台设备的找回请求先扣着，提示里给“验证这台设备”按钮；签名同步到本地后自动发出。发送方因设备没验证而拒绝分发的消息也会请求重发。

### 界面翻新

- 2026-09-29 维护者要求“全部翻新”：流程减到必要的几步，全站一套组件，说人话。设计在 [specs/interface-renewal/design.md](./specs/interface-renewal/design.md)，按文档分步交付，进度记在它的“状态”一节。
- 视觉仍按 [游戏大厅界面重做](./specs/game-lobby-refresh/design.md) 的令牌与形状规则；那次只换了外观，这次改流程、文案和组件。
- 第 2 步（一屏的“接入 Agent”对话框）和第 3 步（新的“我的 Agent”、这台电脑、去掉浮动面板和 `/onboarding`）要在同一个版本里发布。
- `/onboarding` 已去掉：桌面端没有默认 Agent 时连接服务照样运行，MCP 和命令行的 Agent 都走本机会话，不依赖默认 Agent。
- 浏览器验收里桌面端的“我的 Agent”页用 `e2e/fixtures/my-agents.html`（加 `?browser` 是网页端），`window.__agentRoomFixtureControls.arriveAgent()` 模拟一个 Agent 接走接入对话框挂着的人物。
- 设置页在 `/settings/<分节>`（通用 / 安全 / 这台电脑 / 关于）；浏览器验收用 `my-agents.html?settings=<分节>`，安全一节用 `security-center.html`。新的浮层提示一律放进根布局的提示栈（`ToastStack` + `Toast`），别再单独 `position: fixed`。
- 没进房间时的页面（连接、登录没完成、进大厅、房间打开中或打不开、找不到页面、配置出错）共用 `shared/ui/entry-shell.tsx` 的 `EntryShell` + `EntryCard`：一张居中卡片，错误码和地址收进“详情”，出错给“回到房间”。
- 界面翻新 6c 改了登录主题（`infra/identity/`）：Alpha 57 的实机验收要新设备授权，发版前提醒维护者准备批准设备码。登录页的标志由 `tools/sync-brand-assets.mjs` 从网页端同步。
- “新建房间”“换个房间”只有 `RoomActions`（`features/room-directory`）一个入口组件，房间菜单、“房间”页和夹具都用它。浏览器验收的 `lobby-scene.html?features=1` 里有两个待答复的邀请（Research lab、Budget review），加入和拒绝都能真走一遍。
- 房间设置是房间页上的一个对话框（`RoomSettingsDialog`），不在房间菜单的抽屉里。`lobby-scene.html?private` 把夹具房间当成你是房主的私人房间，四节都能看到；浏览器验收用 `openRoomSettings(page, '节名')` 打开。

### 版本与其他

- Alpha 56 已于 2026-09-29 公开，见 [发布记录](./specs/agent-access/alpha56-release.md)。
  - 包含新请进房间的 Agent 读到加入前的消息 #222–#225（Agent 的和人的都行）、Agent 连不上 Bridge 时在后台拉起桌面端 #221、退出桌面端时 Bridge 有序退出 #220、默认文件日志记下房间密钥重发的结果 #217。
  - 登录相关代码没变，实机验收复用 Alpha 53 那台长期验收设备，没要设备码；版本 PR 合并到网页上线约 57 分钟。
  - 升版本时主检出的 `node_modules` 还是 #215 之前装的，`bump_release_version.py` 刷新许可证清单报缺包；`corepack pnpm@10.28.0 install --frozen-lockfile --force` 后重跑 `python tools/license_inventory.py generate` 即可。
- Alpha 55（2026-09-29）见 [发布记录](./specs/agent-access/alpha55-release.md)：找回解不开的历史 #206/#208/#211、私人房间一键生成网络 Agent 口令 #207 等。
- Alpha 54（2026-09-28）见 [发布记录](./specs/agent-access/alpha54-release.md)：一次性密钥积压修复 #201、解不开的消息提示 #202 等。
- Alpha 53（2026-09-28）见 [发布记录](./specs/agent-access/alpha53-release.md)；长期验收设备是 Alpha 53 那台（当时登录相关代码有变化，重新走了新设备授权）。
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
