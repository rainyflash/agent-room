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
- 新增 MCP 工具时，同步改三处，否则发版的打包冒烟会挂：
  - `tools/plugin.py` 的 `EXPECTED_TOOL_ANNOTATIONS`；
  - `approval-policy.example.toml`，按 Rust 里声明的顺序；
  - `tools/mcp_client.py` 的工具集合与 schema 校验。
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
- 自动回复授权的默认有效期是 30 天（已实现）。
- 只凭网络接入的 Agent（[ADR 0010](./docs/adr/0010-network-agents.md)）有两条维护者已接受的取舍：
  - 公开大厅允许没有账号的网络 Agent；
  - 网络 Agent 凭口令进的私人房间，服务器能读到发给它的消息。
- Mac 版已经过苹果公证。别再在文档里教用户去“隐私与安全性”里放行。

## 当前进度（2026-09-24）

### 只凭网络接入的 Agent

- 设计在 [specs/network-agents/design.md](./specs/network-agents/design.md)，实现以它为准，进度记在它的“状态”一节。
- 第 1、2 步已完成。
- 第 3 步（网络 Agent 凭口令进端到端加密的私人房间，服务器替它管加密存储）：
  - 已合并：3a #170、3b #171、3c #172。
  - 还开着：3d #173（加密发言）、3e #174（存储丢失重建）。
    - 这两个是叠加 PR：#173 合并后，把 #174 改成对 main 并 rebase。
    - 3c 和 3d 必须在同一个版本里发布。
  - 下一步 3f：
    - 私人房间里，网络 Agent 的标记加上“服务器代收发”；
    - 生成或更换口令时、房间的加密说明里，提示“网络 Agent 由服务器代收发”；
    - 在 `tools/headless_acceptance.py` 里做真实验收：凭口令进私人房间、与本机 Agent 加密收发、控制面重启后仍能解密、删掉存储后能重建。
- 还欠一件小事：网络 Agent 停用时，应把它在私人房间里的 Agent 成员记为已移出。现在得等房主手动移出。

### 版本与其他

- Alpha 51：生产服务端已部署，网络 Agent 总开关已打开。公开发行卡在实机验收的新设备码，要等维护者在本机批准。
- Alpha 52 将包含远程 MCP #167、接入面板 #168 和第 3 步。
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
