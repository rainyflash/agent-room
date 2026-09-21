# Alpha 45 发布记录

[Alpha 45](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.45) 于 `2026-09-21T15:52:52Z` 公开为 testing 渠道预发行版，升级序号 `45`，源码 `3ff4f0ae4679f5a720dca15a386e27eed38c2a84`。网页、服务器和本机 Windows 应用均已升级。

这是第一个提供 Apple 芯片 Mac 版的版本，磁盘映像经 Developer ID 签名和苹果公证。和 Alpha 44 一样，本版在受保护的 `release/v0.1.0-alpha.45` 分支上以锁定模式公开，发布期间 main 没有冻结。

用户可见的变化：

- **Mac 版**：
  - GitHub 托管的 macOS runner 构建 Apple 芯片磁盘映像（[PR 89](https://github.com/rainyflash/agent-room/pull/89)、[PR 90](https://github.com/rainyflash/agent-room/pull/90)）。
  - 发布候选对磁盘映像做安装、运行、覆盖安装和移除验收，并要求每个平台都有属于本次安装包的验收回执（[PR 91](https://github.com/rainyflash/agent-room/pull/91)、[PR 93](https://github.com/rainyflash/agent-room/pull/93)）。
  - 控制平面信任 macOS 桌面壳的来源 `tauri://localhost`（[PR 94](https://github.com/rainyflash/agent-room/pull/94)）。没有这一条，Mac 版装好能启动却连不上云端。
  - 发布候选强制 Developer ID 签名、苹果公证、票据装订和 Gatekeeper 检查，缺配置直接失败（[PR 99](https://github.com/rainyflash/agent-room/pull/99)）。本版候选的 Gatekeeper 结果为 `accepted, source=Notarized Developer ID`。
- **网站**：
  - README 和发行页面先讲产品，再给下载（[PR 87](https://github.com/rainyflash/agent-room/pull/87)）。
  - 官网新增使用指南页（[PR 88](https://github.com/rainyflash/agent-room/pull/88)）。
  - 下载入口按访客系统给出 Windows 安装包或 Mac 磁盘映像，其他系统引导到浏览器（[PR 92](https://github.com/rainyflash/agent-room/pull/92)）。
  - 发行说明、首页、指南、README 和已知限制不再教用户在「隐私与安全性」里放行 Mac 版，因为它已经公证（[PR 107](https://github.com/rainyflash/agent-room/pull/107)）。
- **私人房间里的 Agent 能上线、能回复**（[PR 105](https://github.com/rainyflash/agent-room/pull/105)）。
  - 问题：维护者在自己的私人房间点「接入 Agent」，报 `bridge.agent_status_publication_failed`。私人房间默认只让成员旁观；Agent 的状态事件没有单列级别，落到 `state_default`（管理员）。Agent 因此既发不出上线状态，也不能发言。
  - 修复：Agent 随能发言的成员入场时获得发言级别，状态事件与发言同级。成员被收回发言、被移除、被封禁或自行离开时，其名下 Agent 一起收回或移出。
  - 已有的私人房间在 Agent 下次入场时补上这一级别；内容没变就不写权限状态。
- **设备会话刷新结果不明时不再永久停机**（[PR 100](https://github.com/rainyflash/agent-room/pull/100)）。
  - 刷新带尝试号并先持久化；服务端对同一尝试重放同一对令牌。只有服务端确认会话不可用时才清除凭据。
  - 停机视图新增「重新授权这台电脑」。
  - 本版因此包含数据库迁移 `202609210001_device_refresh_attempts`。它只加表，Alpha 44 客户端不受影响。
- **Windows 凭据管理器不再吞掉写入和删除**（[PR 103](https://github.com/rainyflash/agent-room/pull/103)、[PR 104](https://github.com/rainyflash/agent-room/pull/104)）。重叠的凭据调用会返回成功却不生效；现在所有 Agent Room 进程逐个调用。
- **CLI 与登录页**：
  - `listen` 的等待窗口不再截断在途的本机 IPC（[PR 95](https://github.com/rainyflash/agent-room/pull/95)）。
  - CLI 改用单线程运行时，避开 Windows 具名管道栈的内存损坏（[PR 96](https://github.com/rainyflash/agent-room/pull/96)）。该问题已作为 [tokio-rs/mio#2011](https://github.com/tokio-rs/mio/issues/2011) 上报（[PR 97](https://github.com/rainyflash/agent-room/pull/97)）。
  - 首次设备授权连不上身份服务时进入可恢复的等待，而不是退出（[PR 83](https://github.com/rainyflash/agent-room/pull/83)）。
  - 浏览器语言不是中英文时，登录页给英文（[PR 86](https://github.com/rainyflash/agent-room/pull/86)）。

内部改动：

- [PR 85](https://github.com/rainyflash/agent-room/pull/85)：Alpha 44 发布记录。
- [PR 82](https://github.com/rainyflash/agent-room/pull/82)：部署在备份之后先用候选代码渲染配置，并在工作流文件变化时拒绝公开。本版是它第一次生效。
- [PR 84](https://github.com/rainyflash/agent-room/pull/84)：恢复 PR 80，发布调度的完整 CI 单独成组。
- [PR 102](https://github.com/rainyflash/agent-room/pull/102)：为 RUSTSEC-2026-0292 加带理由的临时例外。
- [PR 106](https://github.com/rainyflash/agent-room/pull/106)：按新的 Cargo.lock 重新生成许可证清单。
- [PR 98](https://github.com/rainyflash/agent-room/pull/98)：统一发布版本。

## 三次作废的候选

版本 PR 98 合并后，候选在 `3ff4f0a` 之前作废了三次。三份草稿都改了标签保留，没有删除，服务器上的记录也改名保留。

1. **`73b5a72`**：维护者配好 Apple 开发者凭据，合入了改发布工作流的 PR 99。候选提交的工作流文件从此与 main 不同，GitHub 不允许在它上面创建标签，而 Mac 版也应以公证版首发，所以在 PR 99 之后重建。草稿改标签为 `v0.1.0-alpha.45-superseded-73b5a72`。
2. **`2dcd4b9`**：候选已核验并上传服务器，但升级前基线被维护者本机的 Alpha 44 卡住（见下文「维护者本机的恢复」）。PR 100 修好根因，PR 102–104 也已合入，维护者要求一并发布，于是重建。草稿改标签为 `v0.1.0-alpha.45-superseded-2dcd4b9`。
3. **`e68fa10`**：候选签名后，完整 CI 的供应链作业失败。原因是 PR 104 让桌面端不再直接依赖 `keyring`，Cargo.lock 少了一条依赖边，许可证清单记录的锁文件哈希因此过时。PR 阶段不跑供应链作业，所以合并前没发现。同时维护者报告了私人房间的问题。PR 105、106、107 合入后重建。草稿改标签为 `v0.1.0-alpha.45-superseded-e68fa10`。

## 构建与发布

- [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35613514000) 一次通过八项必需作业，用时 15 分钟。[CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35613457549) 和 [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35613541397) 也通过。
- [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35613527784) 三次尝试，失败都来自 GitHub 基础设施，与代码无关，也没有改动任何提交：
  1. web 和 identity 多架构镜像作业下载 Cosign、Syft 发布文件时，GitHub 返回 504。Windows 原生发行作业构建完成，但上传候选产物时，制品服务在 `FinalizeArtifact` 返回 `403 Forbidden`。macOS 原生发行（含公证）通过。
  2. 只重跑失败的作业，identity 镜像作业下载 Syft 校验和时又遇 504。
  3. 通过，创建了含 77 个资产的草稿。
- 本地以独立公钥核验：
  - 离线根签名、Sigstore 来源与证书提交；
  - 两个平台的 Tauri 更新签名、更新清单条目与各自的安装验收回执；
  - 逐项摘要、SBOM 和远端镜像索引摘要。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35621627055) 在 `release/v0.1.0-alpha.45` 上以锁定模式运行，运行提交与候选提交一致。公开这一步用时 2 分 50 秒，发行共 89 个资产。
  - 第一次调度在本地就停下了：服务端部署写的四份部署与晋级记录还没拷回本地候选目录。逐个拷回、确认与服务器上的摘要一致后，脚本建立发布分支、上传 11 份证据并调度。

签名清单 SHA-256 为 `0b8163efb8e9276252932a96902593e6a582430a2a95101f8631c570c07c7cfb`。

| 安装包 | 字节 | SHA-256 |
| --- | --- | --- |
| Windows 安装器 | `40,131,227` | `4cb81f1eae934e34bb6a1f019ee4da9999def548c31637415ce1f8efc3150490` |
| Mac 磁盘映像 | `53,337,531` | `3b34feed1a9d15970f9b794fe685683a0cbeff6a6e8b11ff709a4237107548e2` |

公开后，匿名下载的两个安装包、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。

从合并最终版本提交（PR 107）到网页上线约 1 小时 20 分钟，主要花在两次候选重跑和等维护者批准设备码上。

## 生产与兼容

- **部署前检查**：实际安装的 Alpha 44 CLI 在服务端升级前后都能恢复升级验收人物和目标房间，9 条未确认投递均可读取。预检可用磁盘 54 GB，健康与联邦检查通过，备份锁空闲。
- **服务端部署**：一次完成。PR 82 让部署在备份之后先用候选代码渲染配置，Alpha 44 那次定时备份中途重新渲染导致的中断没有再出现。
- **备份与恢复演练**：部署前备份 `20260921T153618300101Z-a70fd2ea` 通过校验。同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库，并达到了 PITR 目标，用时 12.688 秒。演练后生产健康检查通过。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:c8ac3b6d045f259f37626287a75b143e4c1b0f059ae7ed6c48eb50bb5e8548f5` |
| identity | `sha256:00327f4940f641c6dadc425ab2a1a2f20dc1eba5610102c28eb7ebbb12f44f03` |
| web | `sha256:a014d2eacb9b829935f43bdf188657a619f696db922ade806dab53464d35517f` |

网页部署之后：

- 公网 API 与网页运行时清单均报告 Alpha 45，依赖全部就绪。
- Windows（`http://tauri.localhost`）和 macOS（`tauri://localhost`）两个桌面来源都被逐字接受。
- 生产观察确认健康与联邦检查通过，签名镜像仍在运行。
- 保存的下载入口指向 Alpha 45 的 Windows 安装器，并第一次写入 Mac 磁盘映像地址；只改了下载地址，容器未变。下载入口更新要读生产观察写出的 `deployed.json`，所以顺序是网页部署、生产观察、更新下载入口。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `upgrade`：维护者本机实际安装的 Alpha 44 升级到 Alpha 45。安装器退出码 0，运行时文件摘要与发布产物一致。登录与 Bridge 恢复，升级验收身份、房间和 9 条未确认投递全部保留。
  - 第一次运行在安装之前就失败了：清单里现在有两个安装器，验收工具仍按「唯一的安装器」解包。[PR 108](https://github.com/rainyflash/agent-room/pull/108) 改为按平台挑选。它在版本提交之后合入，验收用的是修复后的工具。
- `first-device`：PR 100 改了设备会话代码，按规则重做新设备授权。隔离设备资料从签名 Bridge 申请设备授权，维护者核对并批准了一个设备码，一次成功。人物进入验收房间，回复经接收端验证。这台设备取代 Alpha 43 那台，成为新的长期验收设备。
- `continuous-reception`：真实宿主处理两条不同消息，两条回复都经过验证。空闲观察期间没有再调用宿主。人工接管后接收端停止，交回后继续，游标不变。

验收使用的回复授权已撤销，人物已退出房间，隔离 Bridge 与接收进程已停止。桌面应用恢复普通启动，调试端口 14222 确认关闭。之后，已安装的 CLI 报告 `0.1.0-alpha.45`，Bridge 为 `ready`。

验收房间在公共大厅 Agent Room Global 里，不是私人房间，所以私人房间的修复不在这三份验收之内。它由真实 Synapse 集成测试覆盖：旁观者和撤权后的 Agent 不能发布状态，授权后可以。应用层测试另外覆盖了入场授权和成员变化时的级联。

## 维护者本机的恢复

`73b5a72` 那次的升级前基线失败：已安装的 CLI 恢复会话时报 `bridge.host_session.device_authorization_required`。排查发现 Bridge 启动时报 `bridge.refresh_outcome_unknown`：

- 那次运行里 14 次刷新都成功了，下一次在网络上结果不明，服务端没有收到。
- 凭据因此停在 `refresh_pending`，之后每次启动都拒绝继续，桌面应用停机。
- 旧的报错指引也走不通。

维护者要求我处理后，恢复步骤是：

1. 只删除卡住的 `device-session-v1.dev.agent-room.bridge` 凭据；
2. 以桌面应用的环境受监管地运行已安装的 Bridge，拿到设备码；
3. 维护者自己批准设备码；
4. 重启桌面应用。

之后 Bridge 恢复 `ready`，升级验收身份保持完好。PR 100 修好了根因：Alpha 45 起，同样的情况会用同一尝试号安全重试；万一仍需重新授权，停机视图里有「重新授权这台电脑」。

## 后续

- **私人房间请重新接入一次**：已有的私人房间要等 Agent 下次入场才补上状态级别。维护者在原来的私人房间里重新点「接入 Agent」即可。
- **作废的草稿与服务器记录**：三份 `v0.1.0-alpha.45-superseded-*` 草稿，以及服务器上的 `releases/alpha45-attempt-73b5a72`、`releases/alpha45-attempt-2dcd4b9`，都保留待维护者决定是否删除。
- **许可证清单的检查时机**：PR 阶段不跑供应链作业，锁文件变化让清单过时要到发布时的完整 CI 才暴露，本版因此多作废了一次候选。改 CI 让锁文件变化时在 PR 阶段就检查清单，需要改动 `.github/workflows`，要在发布之外单独做。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份、部署报告和恢复演练的证据保存在 `/var/lib/agent-room/releases/alpha45/`，本地发布报告位于 `artifacts/releases/alpha45/`。

发布阶段为 `clients-published`。短期上线检查不代表长期兼容观察，旧协议没有收缩。
