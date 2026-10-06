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
- clippy 开了 `too_many_lines`（100 行），函数太长就拆出辅助函数。测试函数也算。
- 合并到 main 的数据库迁移不再改（SQLx 记着每个迁移的校验和），要改就新加一个。
- 设了自动合并的 PR，CI 一绿就会合。要补的提交（比如在 CLAUDE.md 里记上 PR 号）先推上去，再设自动合并。#316 就是补的提交还没推上去就合了，后续改动只好另开 #317。
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
- “macOS 客户端运行时原生检查”（#320）也只在派发 `suite=all` 时跑，PR 上不跑，因为 macOS 编译慢。它在 Mac 上跑和 Windows 相同的 `node tools/desktop.mjs native-check`，发布调度要求它通过。改 Bridge 本机连接、平台存储、桌面端这类跟系统打交道的代码时，派发一次看它。
- 改 `apps/desktop/src-tauri/windows/` 下的安装器钩子时，PR 上会跑“Windows 安装器钩子”工作流（`tools/windows_installer_hooks.py`）：编译精简安装器，实跑运行中覆盖安装与卸载，一分钟左右。
  - 它按路径触发，不是必需检查，红了同样不能合。
  - 本机跑要加 `--isolated`；不加会拒绝运行，免得结束正在用的 Agent Room。
  - 升级 `@tauri-apps/cli` 时，同步更新工具里固定的模板提交和哈希，单元测试会提醒。
  - 占位程序是 NSIS 编出来的，运行时一直以不许删改的方式开着自己，所以运行中挪不开。“桌面端挪开”这条路要在桌面端已退出的场景（WebView 还开着本机数据）里查；真正的桌面端运行时能改名，候选上的真实安装验收会查。
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
- **Synapse 默认的发言限速。** 生产配置没写 `rc_message`，用的是默认值：每个 Matrix 用户连发 10 条以后每 5 秒才放一条，人和 Agent 都一样。网络 Agent 被挡下时控制面回 429 `network_agent.rate_limited` 带 `Retry-After`（#311 之前回的是 503）。测试里要一个人连发十几条，就分给几个人发，或者按 `Retry-After` 等；2026-10-05 无头验收的积压就是这样改成六个人各说 10 条的。
- **Synapse 没接 MAS 时，已有签名身份的账户换签名身份一律要交互认证。** 管理接口 `_allow_cross_signing_replacement_without_uia` 只在接了 MAS 时起作用，只有应用服务的请求例外（MSC4190）。所以人的设备自动签名重建签名身份时，新签名公钥由控制面以应用服务身份冒充本人上传；应用服务注册为此有一个覆盖所有本地用户的非独占命名空间（ADR 0011 的“修订”）。
- **matrix-js-sdk 的 `bootstrapCrossSigning` 看到本机有签名私钥就不上传公钥。** 上次上传被打断（页面跳走）时，本机留着私钥、服务器上却没有签名身份，它也照样跳过。账户还没有签名身份时要用 `resetEncryption` 从头建，别用 `bootstrapCrossSigning`。
- **本机加密存储丢了的设备只能换设备号。** Agent 的加密库遇到“同一个设备号换了签名公钥”一律不认（matrix-sdk-crypto 的 `SigningKeyChanged`），这台设备再也拿不到房间密钥，消息全都解不开。所以网页端和桌面端恢复会话时，加密库起来以前先问服务器这台设备记着的签名公钥，起来以后跟本机的比（`deviceKeysReplaced`），对不上就注销这台设备、重新登录拿新设备号。只在本机加密库持久保存时比：放在内存里的每次都是一套新密钥。
- **升级时别让桌面端在换文件的当口启动。** Agent 的 MCP 和命令行连不上 Bridge 会在后台拉起桌面端（#221）。安装器停 Agent Room 的当口被拉起的旧版，会和正在退出的 WebView 抢同一份本机数据（推测 Chromium 打不开就整库删掉重建），升级后网页存储（加密库、登录）被清空：Alpha 57、62 都遇到过。所以安装器钩子：
  - 先在安装目录占住标记 `installer-running.lock`（不许别人打开、关掉就删），再把桌面端程序改名为 `agent-room-desktop.exe.old`，然后才结束进程；
  - 等到四个进程都退出、程序都能写、`%LOCALAPPDATA%\dev.agent-room.desktop\EBWebView\lockfile` 也放开了才换文件；
  - 装完删掉挪开的程序、放开标记；没装成就挪回原处。
  - 桌面端启动时看到标记被占着就直接退出（`installer_marker.rs`）。改安装流程时这几样别拆。
- **Windows 具名管道会踩坏堆。** tokio 的客户端在“丢弃连接”与“I/O 驱动处理同一管道”并发时会出这个问题（上游 mio#2011）。#145 起，本地客户端连接都放在专用的单线程运行时线程上跑；上游修好之前别拆。
- **Windows 凭据管理器会吞掉重叠的写入和删除。** 产品代码经 `SystemCredentialStore` 逐个调用，新代码别直接用 `keyring`。
- **macOS 不让给还没绑定的 socket 设权限。** interprocess 的 `ListenerOptionsExt::mode` 在绑定前调 `fchmod()`，Linux 支持，macOS 一律返回 EINVAL。Bridge 从第一版起就用它，每台 Mac 授权完都报 `bridge.ipc_bind_failed`、建不起本地连接，Linux 和 Windows 的测试照样全过。#320 起绑定后再用 `restrict_socket_to_owner` 收紧到 0600（运行目录先验过 0700），别再用 `mode()`。
- **生产对象备份用 `rclone/rclone`**（#271，Alpha 59 起）。MinIO 把开源项目归档了，`minio/mc` 的镜像和程序都已下架；Alpha 58 时临时重建的同名镜像和 `/root/mc-rebuild` 已在 Alpha 59 上线后删掉。`object-backup` 这类按需运行的容器平时没有容器在用，`docker image prune -a` 会把它们的镜像一起删掉，清镜像时要排除。
- **真实 Synapse 测试里的加密房间。** 参与者要用全新的受管账户：种子账户每次登录都会得到一台缺私钥的新设备。
- **聊天消息的标题和摘要别直接截正文。** 截出来会带换行，IPC 校验不收控制字符，多行消息就发不出去（`bridge.ipc.message_title_invalid`）。一律用 `IpcSendMessageRequest::chat_title_and_summary`，它先把正文压成一行。消息正文收换行和制表符，不收回车；命令行发之前把 CRLF 统一成换行。
- **本机 Bridge 和网络 Agent 网关共用 matrix-adapter 打开客户端的那段**（`restore_with_handoffs` → `handoff_connection_from_client`），挂在那里的功能网络 Agent 也有。Alpha 56 的“找回加入前的消息”就这样让网络 Agent 也请别人重发加入前的房间密钥，服务器因此读得到加入前的消息；#316 起网关用 `without_room_key_requests()` 关掉。只给本机的功能要加配置开关，网关那边关掉。

## 产品决定（已定，别再问）

- 核心体验是“一个按钮把 Agent 请进来就能聊”，参照桌面端的“接入 Agent”对话框，流程要一步到位。
- Agent Room 是通用 Agent 软件，不做只服务某个 Agent 应用的功能、界面或提示词；后台回复按宿主恢复任务属功能需要，保留。
  - 接入只有网络、MCP、命令行三种通用方式，MCP 只给一份通用 JSON。
  - 给 Claude Code 装技能、按应用一键写 MCP 配置、Codex 插件发行包都已去掉，别加回来。
- 自动回复授权的默认有效期是 30 天（已实现）。
- 只凭网络接入的 Agent（[ADR 0010](./docs/adr/0010-network-agents.md)）有两条维护者已接受的取舍：
  - 公开大厅允许没有账号的网络 Agent；
  - 网络 Agent 凭口令进的私人房间，服务器能读到发给它的消息。
    - 只到“发给它的”为止（维护者 2026-10-05 定，#316）：它进房间之前的加密消息不给它读，网关不请别的设备重发加入前的房间密钥，只用 `gaps` 的 `undecryptable_before_join` 告诉它前面有一段解不开。别给网络 Agent 加“找回加入前的消息”。
- Mac 版已经过苹果公证。别再在文档里教用户去“隐私与安全性”里放行。
- 人的设备自动签名，不要恢复密钥（[ADR 0011](./docs/adr/0011-automatic-device-signing.md)，维护者 2026-10-02 定）：服务器保管账户签名用的钥匙，任何设备登录就自动签名、自动找回加密历史；界面上不再有恢复密钥、恢复口令和设备核对，别加回来。代价是部署方能替账户签设备，维护者接受。设计在 [specs/device-signing/design.md](./specs/device-signing/design.md)。

## 当前进度（2026-10-06）

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
- 加入前解不开的一段 #316：网关同步时看到 Agent 自己加入，服务器早于加入收到、又解不开的就是这一段，挂在这个房间之后第一条进收件箱的消息上（`gaps` 的 `undecryptable_before_join`，没有 `afterEventId`）。
  - 每次加入只说一次（#317）：房间上记着说到了哪一次加入（服务器收到加入的时间）。被移出以后又凭口令进来是新的一次，再说一次。
  - 记在新加的列里（`network_agent_room` 的 `before_join_gap_status`、`before_join_gap_joined_at_ms`，`network_agent_inbox.before_join_gap`），不写进 `gap_reason`：旧版控制面读到不认识的原因，整个收件箱都读不出来。
  - 私人房间那轮无头验收也查它：网络 Agent 进来之前本机 Agent 先说一句，它收到的第一条要带着这一段；发送方 Bridge 日志里不能有应它的请求重发房间密钥的记录。
  - 维护者 2026-10-05 让清掉 Alpha 56 到 Alpha 62 期间网络 Agent 请来的加入前房间密钥。先只读查了生产：只有 3 个网络 Agent 进过加密房间，都在同一个房间；它们加入之前，房间里没有加密消息，密钥备份也是空的。没有要清的，什么都没删。

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
- 第 5 步 #294：Agent 发现主人核对过又换了签名身份时，自动撤掉旧的核对、记住新身份（matrix-adapter 的 `AgentOwner`，Bridge 恢复连接时交给它）。不然以前跟主人核对过安全码的 Agent 会整个房间发不出消息。
- 第 2 到第 5 步和 #297（第一台设备上传公钥被打断后从头建）都随 Alpha 61 发布。
- 维护者桌面端 2026-09-30 起本机加密存储被重建过、设备号没换，Agent 不认“同一设备号换了密钥”，房间里的消息全部解不开。Alpha 61 发布当晚退出再登录、换了新设备号才好。之后再遇到，恢复会话时会发现本机和服务器上这台设备的签名公钥对不上，自动注销、换新设备号登录（见上面“代码里的坑”）；网页端也会请浏览器持久保存本站数据。
- 2026-10-05 升 Alpha 62 时又遇到一次，这次由 #302 接住了：注销旧设备、打开系统浏览器重新登录。但浏览器那边 15 分钟没走完，桌面端就停在“连接暂时未完成”，房间列表上看不出来，进房间才显示“消息还没接通”（`lobby.matrix_unavailable`）。经过见 [Alpha 62 发布记录](./specs/agent-access/alpha62-release.md)。
  - 两次被清空都是升级的时候：新版第一次启动前几秒，旧版在安装中途又被拉起了一次（桌面端日志 `AgentRoom/Bridge/logs/desktop.log` 的“桌面端启动 version=”）。这是 #221（Agent 连不上 Bridge 就后台拉起桌面端）上线以后才有的。到底哪一步清掉了 WebView 的 IndexedDB，还没复现确认。
  - 界面这边 #314 做了：
    - 账户登录着、消息却没连上而且要人动手时，提示栈里常驻一条：等浏览器登录时给“重新开始登录”，出错时给“重新连接”；
    - 房间页“消息还没接通”换成同一套说法，按钮真的去重连；
    - 桌面端再开始一次登录时，还在等的那次让位，不用再等满 15 分钟；
    - 夹具 `my-agents.html?matrix=signin|failed` 能看这两种提示。
  - 安装器这边 #315 做了：
    - 停 Agent Room 前先占住标记、把桌面端程序挪开，Agent 拉不起旧版；
    - 等 WebView 放开本机数据再换文件；
    - 新版桌面端看到标记就退出（见上面“代码里的坑”）。
    - 钩子是新版安装器带的，从 Alpha 62 升上去那次就生效。升级后桌面端还掉登录的话，先看 `desktop.log` 里安装那几秒有没有“桌面端启动”或“安装器正在换”。
  - 2026-10-06 维护者桌面端从 Alpha 62 升 Alpha 63，#315 第一次在真实升级里起作用：登录和本机加密存储都保住了，安装那几秒日志里只有新版的一次启动。

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
  - 本机 MCP 的服务说明（`SERVER_INSTRUCTIONS`）已经 1535 字节，测试卡在 1536 以内，要加话得先删别的。
- 本机收件箱（第 4 步）：
  - 4a #299：Bridge 按房间记确认位置（IPC 4.4 的 `AckInbox`，读收件箱和等消息带 `fromAck`，消息库迁移 0007）；
  - 4b #300：MCP 的 `agent_room_ack`，命令行的 `ack` 交给 Bridge 记；后台回复处理完一批也在 Bridge 上确认；
  - 4c #301：只要点我的（`mentionsOnly`，放在三种接入共用的 `WaitParams`）；
  - 4d #303：补不回来的一段（`gaps`）。往回补翻到上限没接上、或者没给往回翻的令牌，记进消息库的 `message_timeline_loss`（迁移 0008），读到它后面那条时报。只做了 `too_many`；后台回复交给宿主时也带上 `gaps`（6b）。
- 网络接入（第 5 步，拆成 5a、5b-1、5b-2、5c）：
  - 5a #304：收件箱每个房间留 500 条没确认的，等消息加 `roomId`、`mentionsOnly`，回答加 `remaining`，确认加可选的 `roomId`；停用后不再写收件箱，记下离开房间时删掉留下的。
  - 5b-1：消息记录 `network_agent_message`（每个房间最近 500 条，自己发的、确认过的也留，和收件箱同一次写入），`replyTo` 从消息记录里补全，HTTP 的按 ID 取（`/me/messages/lookup`）和翻房间（`/me/rooms/{roomId}/messages`）。
  - 5b-2：远程 MCP 的 `agent_room_get_messages`、`agent_room_room_messages`（和本机同名同参数，没有 `sessionId`），网络接入的收件箱打开长消息截断。
  - 5c：同步时房间被截断就往回补（每个房间一次凑够 500 条为止），补不全的记在那段之后第一条收件箱消息上（`gap_reason`、`gap_after_event_id`），交出它时带 `gaps`；网络接入没有会话，确认之前每次交都再说一遍。往回翻暂时读不到时这次同步不算数，同步位置不动。
- 无头验收有一轮看之前的消息和积压（`verify_reading_and_backlog`，第 6 步的 6a）：被点名后按 ID 取原文、看前后、往前翻（HTTP 和远程 MCP），网络 Agent 积压 60 条回来一条不少。只在派发 `suite=all` 时跑。
- 三份说明一个说法（6b）：本机 MCP 的服务说明、远程 MCP、`agents.md`、`agent-room guide` 都是“等消息 → 处理完确认 → 之前的用 `agent_room_room_messages`（命令行 `history`、`around`）翻、全文用 `agent_room_get_messages`（命令行 `show`）按 ID 取”。后台回复交给宿主的这一批带上 `gaps`（有才给），提示里说那段取不到、别当成对话是连着的。
- 接入说明有两个地址：`/agents.md` 和 `/agents.txt`，同一份内容，都按 `text/plain` 发。接入对话框给 `.txt`：有的网页读取器按网址结尾猜类型、不看响应头，见了 `.md` 就当 markdown 拒收（维护者 2026-10-02 遇到过）。
- 只会浏览网页、发不了请求的聊天助手只能靠它所在的应用加 MCP 连接器（`{API}/mcp`）接入，有的应用要付费版、有的根本没有；别再想“把加入和发言做成能直接打开的链接”，令牌会进 URL（#251 的 PR 描述里有完整取舍）。

### Mac 版

- 2026-10-05 维护者的 Mac（macOS 15.7.7，Apple 芯片）装上 Alpha 62、授权完以后，一直停在“这台电脑的连接停了”：Bridge 每次启动都报 `bridge.ipc_bind_failed`，原因见上面“代码里的坑”。所有发布过的 Mac 版都这样。
- 排查办法：维护者在那台 Mac 上开一个 Claude Code 会话，凭口令进私人房间；编码 Agent 在房间里请它跑只读命令、贴日志。日志在 `~/Library/Application Support/Agent Room/Bridge/logs/`（`desktop.log`、`bridge.log`）。
- 修复 #320 随 Alpha 63 发布，同时加了“macOS 客户端运行时原生检查”。2026-10-06 那台 Mac 升到 Alpha 63，是 Mac 版第一次真机连通，经过见 [Alpha 63 发布记录](./specs/agent-access/alpha63-release.md)：
  - 升级由那台 Mac 上的 Claude Code 按房间里的步骤做，维护者在 Mac 上同意；
  - Bridge 两秒内 Authorized，用的是之前保存的授权；`runtime/bridge.sock` 是 `srw-------`；
  - 命令行 `doctor` 立即 `ready`，钥匙串没弹窗。
- 还没有默认 Agent 时，桌面端问“默认 Agent 是谁”（`get_self`）得到 `bridge.agent_runtime_unavailable` 是正常的，Windows 上也一样。Alpha 63 及以前的 `bridge.log` 里因此有 WARN（启动时一条，之后每十分钟一条）；#324 起只按 debug 记。
- 之后在 Mac 上接入 Agent、后台回复，还可能碰到别的 Mac 专属问题，排查照上面的办法。

### 版本与其他

- Alpha 63 已于 2026-10-06 公开，见 [发布记录](./specs/agent-access/alpha63-release.md)。
  - Mac 版建不起本地连接的修复 #320（CI 加 macOS 原生检查）；升级时别让桌面端在换文件的当口启动 #315，消息没连上时说清楚、能一键重连 #314；网络 Agent 加入前解不开的一段每次加入告诉它一次 #316、#317；命令行和后台回复能发多行消息 #319；两条开发依赖安全公告 #321。
  - 没改登录相关代码，实机验收复用 Alpha 61 那台长期验收设备，没要设备码。
  - 维护者桌面端从 Alpha 62 升上来，登录和本机加密存储都保住了；维护者的 Mac 升级后第一次连通（见上面“Mac 版”）。
  - 版本 PR #322 合并到网页上线约 1 小时，其中约 20 分钟是合并后没及时派发：开了自动合并，却没有等合并的脚本，应用也没来通知。发版时版本 PR 合并要有人盯着，合并后马上派发。
- Alpha 62 已于 2026-10-05 公开，见 [发布记录](./specs/agent-access/alpha62-release.md)。
  - Agent 怎么看房间里的消息全部做完（本机收件箱 #299–#303，网络接入 #304、#306–#308，无头验收 #309，三份说明和后台回复的 `gaps` #310）；本机加密存储丢了换新设备号 #302，退出登录前先叫停自动签名 #305；网络 Agent 被 Matrix 限速时回 429 #311。
  - 没改登录相关代码，实机验收复用 Alpha 61 那台长期验收设备，没要设备码。
  - 升级把维护者桌面端的本机加密存储清空了，#302 注销旧设备、要求重新登录；第一次验收停在“进房间”，维护者重新登录后重跑通过（见上面“人的设备自动签名”）。
  - 版本 PR #312 合并到网页上线约 1 小时 26 分钟，其中约 40 分钟在等维护者重新登录。
- Alpha 61 已于 2026-10-03 公开，见 [发布记录](./specs/agent-access/alpha61-release.md)。
  - 人的设备自动签名（设计 #289，实现 #290、#291、#293、#294、#295），界面上不再有恢复密钥和设备核对。
  - 发布提交上的完整 CI 抓到第一台设备上传公钥被打断后，服务器上一直没有签名身份（#297）。#297 的合并提交成为发布提交，候选重建一次。
  - #295 改了 `tools/prodops/render.py`（应用服务注册），实机验收要了一次设备码；长期验收设备换成 Alpha 61 那台（`agent-room.alpha61.acceptance.fresh-device`）。部署时 Synapse 按计划重启了一次。
  - 版本 PR #296 合并到网页上线约 12 小时 13 分钟，其中约 9 小时是空等：等定时备份的脚本拿 `systemctl is-active` 的退出码当条件，服务停下时退出码是 3，脚本一直没往下走。等待脚本要按输出判断、设截止时间，走开前先看头几轮输出。
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
