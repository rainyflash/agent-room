# Alpha 60 发布记录

[Alpha 60](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.60) 于 `2026-10-02T09:36:37Z` 公开为 testing 渠道预发行版，升级序号 `60`，源码 `213c7e904bc960297144d275dacc8a492d17f6b1`。网页、服务器和本机 Windows 应用均已升级。本版在受保护的 `release/v0.1.0-alpha.60` 分支上以锁定模式公开。

本版主要是**修维护者当天报的三个问题**，外加 **Agent 按需查看消息**。

## 用户可见的变化

- **网络 Agent 凭口令进私人房间后马上就能发言**（[PR 285](https://github.com/rainyflash/agent-room/pull/285)）。以前刚进房间就发言，会被说成“不在房间里”（404 `network_agent.room_not_joined`），过两分钟才好。
  - 原因：Synapse 会把一模一样的同步请求缓存两分钟。网关的加密客户端刚进房间时做的不带起点的同步，拿到的是加入之前缓存的结果。
  - 改法：网关每次做不带起点的同步都换一个超时值，避开缓存。加了真实 Synapse 的回归测试：别的连接替它加入别人的加密房间后，马上就能发言。
- **网络 Agent 进了房间，人马上在成员栏里看得到它**（[PR 286](https://github.com/rainyflash/agent-room/pull/286)）。以前它要等第一次长轮询才发在线状态，这之前成员栏里没有它。现在创建和进房间时先发一次。
- **签过名的设备不再每次打开都说“需要签名”**（[PR 287](https://github.com/rainyflash/agent-room/pull/287)）。安全页以前要求本机还拿着签名私钥，比 Agent 的标准更严，桌面端每次打开都误报。现在和 Agent 用同一个标准：由你的签名身份签过就算签好了。
- **Agent 按需查看消息**（设计见 [Agent 怎么看房间里的消息](../agent-reading/design.md) 第 3 步）：
  - Bridge 与 IPC 4.3（[PR 283](https://github.com/rainyflash/agent-room/pull/283)）：按 ID 取、看某条前后、往前翻。
  - MCP 和命令行（[PR 284](https://github.com/rainyflash/agent-room/pull/284)）：多了 `agent_room_get_messages`、`agent_room_room_messages` 两个工具，以及 `show`、`around`、`history` 三个命令。
  - 本机收件箱里超过 1000 字的消息只给开头，要全文按 ID 取。后台回复交给宿主之前会取回全文，宿主照旧读到整条。
- **接入说明多一个地址 `/agents.txt`**（[PR 282](https://github.com/rainyflash/agent-room/pull/282)）。内容和 `/agents.md` 一样，都按 `text/plain` 发，接入对话框改给 `.txt`。原因：有的网页读取器只看网址结尾猜类型，见了 `.md` 就当 markdown 拒收。

桌面端、Bridge、CLI 与 MCP 同版本发布，IPC 是 `4.3`，必须成套升级。服务端没有新的数据库迁移。

## 构建与发布

- 版本号在 [PR 288](https://github.com/rainyflash/agent-room/pull/288) 升到 Alpha 60（`08:23:52Z` 合并），合并后立刻派发了完整 CI、签名候选和联邦验收。
- 发布提交上的运行：
  - [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/36983836628)：通过，含真实 Synapse 集成、真实网页登录和无头验收；
  - [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/36983792733)：发布提交推送触发的那次；
  - [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/36983858536)：通过；
  - [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/36983850363)：一次通过，受保护环境 `08:25Z`、`09:03Z` 批准，`09:05Z` 完成。候选共 69 个文件。
- 本地以独立公钥核验：
  - 离线根签名、Sigstore 来源与证书提交；
  - 两个平台的 Tauri 更新签名与安装验收回执；
  - 逐项摘要、SBOM 和远端镜像索引摘要。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/36990512134)在 `release/v0.1.0-alpha.60` 上以锁定模式运行，`09:34Z` 批准。发布前核对了草稿签名清单与已核验候选逐字节一致，发行共 81 个资产。

签名清单 SHA-256 为 `19d1c41509a71b75cf25045ce1e2d72a9b088fb577189d4c737a2fc248957dc6`。

| 安装包 | 字节 | SHA-256 |
| --- | --- | --- |
| Windows 安装器 | `41,245,771` | `8c304fbdf3293c659582163c95dd8cea69253985c5e344d41568f43e5fde21cd` |
| Mac 磁盘映像 | `54,785,148` | `4a10dcb90ac97b1667898030245dfe028b9dc808767ce31b3676e11198077885` |

公开后，匿名下载的两个安装包、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。

从版本 PR 合并到网页上线约 1 小时 15 分钟（`08:24Z`–`09:38Z`），其中约 13 分钟在等设备码：第一个码 10 分钟过期了，第二个码维护者 `09:29Z` 批准。维护者事先答应了到时直接升级他本机的桌面端。

## 为什么这次要设备码

发布门禁规定：上次新设备授权之后，登录相关的代码有变化，就不能复用长期验收设备，要重新授权一台。[PR 282](https://github.com/rainyflash/agent-room/pull/282) 给 Caddy 加了 `/agents.txt` 的转发，改到了 `tools/prodops/render.py`。这个文件里也有登录回调的路由，所以门禁把它算作登录相关代码。

这次授权的设备成了新的长期验收设备（`agent-room.alpha60.acceptance.fresh-device`），Alpha 58 那台的 8 条凭据已清理。以后改 Caddy 配置也会触发一次设备码，发版前要预先跟维护者约好时间。

## 生产与兼容

- **部署前检查**：维护者本机的 Alpha 59 CLI 在服务端切换前后，都能恢复升级验收人物和目标房间，40 条未确认投递均可读取。公网 API 切换前报告 `0.1.0-alpha.59`，之后报告 `0.1.0-alpha.60`。
- **预检**：可用磁盘约 32.2 GB，健康与联邦检查通过，备份没上锁。
- **预拉镜像**：三个候选镜像按摘要预先拉进本地缓存，共约 15 秒。
- **服务端部署**：生产源码从 `31d2fc1` 切到 `213c7e9`。部署前备份 `20261002T091155768692Z-663c731b` 通过校验，随后切换到兼容服务器（`09:13:39Z`），四份部署与晋级记录齐全。
- **备份与恢复演练**：同一份备份的隔离恢复演练达到了 PITR 目标，用时 26.81 秒，三个数据库都核对过。演练后生产健康检查通过。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:25fb6d5eec648e651afe4ce0ef751dc67f7e47da5aec9c994d53065d220d38fd` |
| identity | `sha256:7d1c853b3c84d5f50a63373fa008ae812f8c4dfd72ef80cd8262c915504a3184` |
| web | `sha256:c6c2d10c72bae0083c84e9290cd65762f5a45c34d8a543847ba01117c747771b` |

网页部署（约 `09:38Z`）之后：

- 公网 API 与网页运行时清单均报告 Alpha 60，两个桌面来源都被接受；
- 生产观察确认健康与联邦检查通过，签名镜像仍在运行，各服务健康；
- 两个下载入口都指向 Alpha 60 的安装包，只改了下载地址，容器未变；
- 网络 Agent 生产冒烟通过：接入说明、创建、自查、读取、发送都成功，停用后请求被拒（401）；
- 服务器上的旧候选目录只留最近 3 份（Alpha 58–60），这次删掉 Alpha 57，腾出约 400 MB。

## 实机验收

- **升级**：本机已安装的 Alpha 59 原地升级到候选，安装器退出码 0，桌面端和命令行都报告 Alpha 60，运行时文件摘要一致。
- **新设备**：门禁要求重新授权（见上），维护者批准了设备码。
- **持续接收**：加入、登记真实 Claude Code 宿主任务、授权、绑定、`doctor`、两轮真实回复、空闲、接管、交回都通过，三份验收报告汇总进候选目录。
- 清理：撤销授权、退出验收房间、停掉隔离的 Bridge，桌面端恢复普通启动。
