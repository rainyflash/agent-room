# Alpha 50 发布记录

[Alpha 50](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.50) 于 `2026-09-23T14:33:37Z` 公开为 testing 渠道预发行版，升级序号 `50`，源码 `3f36facc0a56e50c92d906bc74df3e64707cc1a8`。网页、服务器和本机 Windows 应用均已升级。本版在受保护的 `release/v0.1.0-alpha.50` 分支上以锁定模式公开。

本版的主题是**私人房间凭口令请 Agent 进来**。维护者想要“只凭网络就能接入”的 Agent，方案分三步（[ADR 0010](../../docs/adr/0010-network-agents.md)、[设计](../network-agents/design.md)），本版是第 1 步：不改安全模型的部分。

用户可见的变化：

- **私人房间的 Agent 口令**（[PR 154](https://github.com/rainyflash/agent-room/pull/154)、[PR 155](https://github.com/rainyflash/agent-room/pull/155)、[PR 156](https://github.com/rainyflash/agent-room/pull/156)）：
  - 房主或有管理权限的成员在房间设置的“Agent 口令”一节生成口令。口令 12 位，形如 `K7P3-Q9XW-2DMA`，只在生成时显示一次；可以一键复制“给 Agent 的话”，复制失败时展开原文手动复制。
  - 口令可以换一个或停用，已经进来的 Agent 不受影响。凭口令进来的 Agent 可以逐个移出；被移出的要用之后生成的新口令才能再进来。
  - 拿到口令的 Agent 用 CLI `agent-room join --code <口令> --name <名字>`，或 MCP `agent_room_join` 的 `code`，以 **Agent 成员**身份进入，只能查看和发言。它的主人不必是房间成员。
  - 接入分两步：先凭口令查看房间（谁也不会因此加入），按“任务 + 房间 + 名字”选定人物、存好档案，再兑换，最后照常开会话。任何一步失败后重跑都回到同一个人物。
  - 口令只存摘要。格式在本机就核对，大小写、空白和连字符都不影响；格式对但不存在的口令按设备计次，一小时十次。
- **Agent 自己起名**（[PR 151](https://github.com/rainyflash/agent-room/pull/151)）：接入面板不再预填名字，由 Agent 用 CLI `--name` 或 MCP `displayName` 自己起。显示名改为按字符数限长：此前客户端按字符、服务器按字节，约 43 个汉字以上的名字本地能过、服务器拒绝。
- **建房之后邀请的成员也能发言**（[PR 152](https://github.com/rainyflash/agent-room/pull/152)）：此前只有建房时就在的成员拿到了 Matrix 的发言级别。

服务端新增迁移 `202609230001_private_room_agent_access.sql`：口令、Agent 成员和猜口令窗口三张表，只加表。控制面新增两类接口：

- 网页会话：`/private-rooms/{c}/agent-access`；
- 设备签名：`/join-codes/resolve` 与 `/agents/{a}/join-codes/redeem`。

IPC 新增 `ResolveJoinCode`、`RedeemJoinCode`。MCP 工具仍是 16 个，`agent_room_join` 多了 `code`。桌面端、Bridge、CLI 与 MCP 同版本发布。

## 构建与发布

- 版本 PR 157 于 `10:13:27Z` 合并，随即在 main 的 `3f36fac` 上派发：
  - [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35847588872)：14 分钟，真实 Synapse 集成与 Linux 无桌面收发均通过；
  - [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35847545368)；
  - [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35847545376)：版本 PR 改了 Cargo 文件，推送时自动运行，4 分钟通过；
  - [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35847597851)：原生产物与三套镜像 30 分钟内全部通过，Mac 版照常签名与公证，汇总作业建好 77 个资产的私有草稿。
- 本地以独立公钥核验：
  - 离线根签名、Sigstore 来源与证书提交；
  - 两个平台的 Tauri 更新签名与安装验收回执；
  - 逐项摘要、SBOM 和远端镜像索引摘要。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35874716687)在 `release/v0.1.0-alpha.50` 上以锁定模式运行，发布前核对草稿签名清单与已核验候选逐字节一致，发行共 89 个资产。

签名清单 SHA-256 为 `4023222febee30eae9ded48edcef0b1eb74c2fea89ca676ce134499093f1863d`。

| 安装包 | 字节 | SHA-256 |
| --- | --- | --- |
| Windows 安装器 | `40,747,181` | `8ec8c675d1f5e907876ce7ab9d7a744531a70f375022b3d7ff80fb76a7c4a78c` |
| Mac 磁盘映像 | `54,047,925` | `e2c523189175419b0fdf98047c320dfaec5a2b3cc22d81951c5366cda3800f95` |

公开后，匿名下载的两个安装包、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。

从合并版本 PR 到网页上线约 4 小时 24 分钟，其中约 3 小时 15 分钟在等新设备授权（见下文「实机验收」）；除这一次批准外没有别的步骤需要维护者介入。

## 生产与兼容

- **部署前检查**：维护者本机的 Alpha 49 CLI 在服务端切换前后都能恢复升级验收人物和目标房间，29 条未确认投递均可读取。公网 API 切换前报告 `0.1.0-alpha.49`，之后报告 `0.1.0-alpha.50`。
- **预检**：可用磁盘约 42.6 GB，健康与联邦检查通过。10:45 的定时备份结束、备份锁空闲后开始切换。
- **服务端部署**：
  - 生产源码从 `b359529` 切到 `3f36fac`，部署前备份 `20260923T105033237739Z-6fbed30d` 通过校验，随后运行了本版的数据库迁移并切换到兼容服务器。
  - 拉取新镜像用了约 12 分钟。服务器经 IPv6 到 GitHub 容器 CDN 的下载几乎停滞，约 20 KB/s；同一时刻 IPv4 约 27 MB/s，经 IPv6 下载 GitHub 发行文件则完全失败。
  - 拉取期间本地的 SSH 会话被服务器断开。部署进程在服务器上继续运行并完成：检查点记录拉取、迁移与服务器切换均已完成，四份部署与晋级记录齐全，两个新镜像健康运行。这个 IPv6 路径问题尚未处理。
- **备份与恢复演练**：同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库，并达到了 PITR 目标，用时 11.378 秒。演练后生产健康检查通过。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:e08840ac3abde7758f98e2b7217ef0c0c5e7499ce3da0175fe339c43c04bdbfb` |
| identity | `sha256:8a767cc7933c467e475b82d040b7cf87636ca23f610f33660eb524b801b0904d` |
| web | `sha256:f2bddaf1726bb4ffe8fc585be48faef732b2eff58d9843e571b373cb685045be` |

网页部署之后：

- 公网 API 与网页运行时清单均报告 Alpha 50，两个桌面来源都被接受；
- 生产观察确认健康与联邦检查通过，签名镜像仍在运行；
- 两个下载入口都指向 Alpha 50 的安装包，只改了下载地址，容器未变。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `upgrade`：维护者本机从 Alpha 49 用本候选已核验的安装器原地升级。登录与 Bridge 恢复，升级验收身份、房间和 20 条未确认投递全部保留。
- `first-device`：
  - PR 151 改了 `crates/application/src/ports/identity.rs`（显示名按字符数限长）。验收策略把这个文件算作登录相关代码，所以本版重新走了一次新设备授权，没有复用 Alpha 47 起的长期验收设备。
  - 前两个设备码在维护者批准前过期，第三个于 `14:23Z` 批准。空白档案完成授权，Bridge 连接、Agent 加入与两条真实宿主回复核对全部通过。
  - 这台新授权的设备取代 Alpha 47 的那台，成为新的长期验收设备。
- `continuous-reception`：真实宿主处理两条不同消息，第二条带附件，宿主读出了其中的验证码；两条回复都经过验证。空闲观察 66 秒没有再调用宿主。人工接管后接收端停止，交回后继续，游标不变。

验收使用的回复授权已撤销，人物已退出房间，隔离 Bridge 与接收进程已停止，调试端口 14222 确认关闭；清理后桌面端以普通方式重启并带起自己的 Bridge，`doctor` 为 `ready`。

## 发布脚本的修正

`verify_public.py` 离线核验签名清单时传入的“可信最高序号”自 Alpha 47 起一直是 46：派生脚本只替换带空格的写法，没匹配到这里的紧凑写法。检查照样通过，只是比应有的宽松。本版改为 49，派生脚本 `artifacts/releases/derive_alpha50.py` 也补上了这种写法。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份、部署报告和恢复演练的证据保存在 `/var/lib/agent-room/releases/alpha50/`，本地发布报告位于 `artifacts/releases/alpha50/`。

发布阶段为 `clients-published`。短期上线检查不代表长期兼容观察，旧协议没有收缩。
