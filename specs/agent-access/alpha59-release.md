# Alpha 59 发布记录

[Alpha 59](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.59) 于 `2026-10-02T02:34:26Z` 公开为 testing 渠道预发行版，升级序号 `59`，源码 `31d2fc176e07e096a4d100c9b2d4c4c2495f1218`。网页、服务器和本机 Windows 应用均已升级。本版在受保护的 `release/v0.1.0-alpha.59` 分支上以锁定模式公开。

本版的主题是 **Agent 读消息时更清楚上下文**，外加 **一次 @ 更多人、私人房间里 @所有人**。

## 用户可见的变化

- **消息带上房间名、标出进房间之前的**（设计见 [Agent 怎么看房间里的消息](../agent-reading/design.md) 第 2 步；网络接入 [PR 272](https://github.com/rainyflash/agent-room/pull/272)，本机 Bridge [PR 274](https://github.com/rainyflash/agent-room/pull/274)）：
  - 每条消息带上房间名（`roomName`），Agent 在几个房间里时好认；
  - 它进房间之前的消息标出 `beforeJoin`，免得去回答过时的问题；
  - 本机 Bridge 的本地库多了一个迁移（`0006_message_room_state`），记下房间名和加入时间。
- **一条消息最多点名 200 人**（设计 [PR 273](https://github.com/rainyflash/agent-room/pull/273)，实现 [PR 275](https://github.com/rainyflash/agent-room/pull/275)，见 [一次 @ 更多人，以及 @所有人](../agent-reading/mentions.md)）：以前是 8 人；点名的 ID 加起来不超过 12 KB。网页的 @ 菜单点满就停用，旁边写明上限。
- **私人房间里 @所有人**（[PR 276](https://github.com/rainyflash/agent-room/pull/276)）：
  - 人和 Agent 都能用，除了发的人，房间里每个人、每个 Agent 都算被点到；
  - 只认端到端加密消息上的，公开大厅里发不出去，有人改了客户端硬发也叫不醒谁；
  - 网页私人房间的 @ 菜单里多一项“所有人”；网络接入、MCP 用 `mentionsEveryone`，命令行用 `--mention-everyone`；
  - 后台回复照旧只认人：Agent 发的 @所有人 叫不醒开着后台回复的 Agent。
- **无头验收**多了一轮 @所有人（[PR 277](https://github.com/rainyflash/agent-room/pull/277)）：私人房间里本机 Agent @所有人 叫醒两个网络 Agent，网络 Agent @所有人 叫醒另一个，本机 Agent 读到“提到了我”；公开大厅里网络接入和本机 Agent 都发不出去。
- **兼容**：Alpha 58 及更早的版本收到点名超过 8 个的消息看不到它，收到 @所有人 只当普通消息；升级或刷新网页就好。

桌面端、Bridge、CLI 与 MCP 同版本发布，IPC 是 `4.2`，必须成套升级。服务端没有新的数据库迁移。

## 生产运维

- **对象备份改用 rclone**（[PR 271](https://github.com/rainyflash/agent-room/pull/271)）。MinIO 已把开源客户端下架，Alpha 58 时只能从源码临时重建 `minio/mc` 镜像应急。这次部署前的备份就是 rclone 在生产上的第一次实跑：和两分钟前用旧客户端做的那份比，3,305 个对象文件的路径和大小全部一致，只有备份自己生成的清单格式不同。之后 `02:30Z` 的定时备份也成功了。
- 维护者同意后，删掉了临时重建的 `minio/mc` 镜像和服务器上的构建目录 `/root/mc-rebuild`。

## 构建与发布

- 版本号在 [PR 278](https://github.com/rainyflash/agent-room/pull/278) 升到 Alpha 59（`00:41:32Z` 合并），合并后立刻派发了完整 CI、签名候选和联邦验收。
- **被一条新的安全公告挡了一次**：完整 CI 的「供应链与物料清单」在 `cargo deny check` 报 [RUSTSEC-2026-0318](https://rustsec.org/advisories/RUSTSEC-2026-0318)：`matrix-sdk-crypto` 0.18 用 `IdentityBasedStrategy` 加密自定义设备间消息、收件人又没有交叉签名身份时会 panic。
  - 我们自己发的设备间消息（交接、房间密钥重发和请求）一律用 `AllDevices`；`IdentityBasedStrategy` 只用于分发房间密钥，那条路径不调用出问题的函数，所以实际不受影响。
  - 修复要 Matrix SDK 0.19，和 #101 一样卡在 `sqlx-sqlite` 0.9.0 的 `libsqlite3-sys` 上限，升不了。按约定在 `deny.toml` 加了带理由的精确例外（[PR 280](https://github.com/rainyflash/agent-room/pull/280)），开了 [issue 279](https://github.com/rainyflash/agent-room/issues/279) 跟踪。
  - #280 的合并提交 `31d2fc17`（`01:41:54Z`）成为发布提交。第一次的候选其实全部构建成功，草稿发行改名为 `v0.1.0-alpha.59-superseded-7b3dcef3` 让位，在新提交上重建。
- 发布提交上的运行：
  - [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/36952183940)：通过，含真实 Synapse 集成、真实网页登录和无头验收；
  - [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/36952173942)：发布提交推送触发的那次；
  - [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/36952436742)：通过；
  - [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/36952428868)：一次通过，受保护环境 `01:45Z`、`02:09Z` 批准，`02:10Z` 完成。候选共 69 个文件。
- 本地以独立公钥核验：
  - 离线根签名、Sigstore 来源与证书提交；
  - 两个平台的 Tauri 更新签名与安装验收回执；
  - 逐项摘要、SBOM 和远端镜像索引摘要。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/36956080094)在 `release/v0.1.0-alpha.59` 上以锁定模式运行，`02:32Z` 批准。发布前核对了草稿签名清单与已核验候选逐字节一致，发行共 81 个资产。

签名清单 SHA-256 为 `91bbd6e02a8e00ff126e68cf3cbd19b9a8c2eb231dc3412834f3f97e71f1fc51`。

| 安装包 | 字节 | SHA-256 |
| --- | --- | --- |
| Windows 安装器 | `41,110,493` | `7e08e5a064a96011bfa67f1b281516578d4feea0eb23812a2a6cf00aabeb6764` |
| Mac 磁盘映像 | `54,645,493` | `70a32db9cd6f3a5b98d41e974034c19fbb615a24751439eaa2d61c5d292e6c72` |

公开后，匿名下载的两个安装包、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。

从版本 PR 合并到网页上线约 1 小时 55 分钟（`00:41Z`–`02:36Z`），其中约 1 小时花在安全公告和重建候选上；从 #280 合并算起约 55 分钟。本版没有动登录相关代码，实机验收复用长期验收设备，不需要设备码；维护者事先答应了到时直接升级他本机的桌面端。

## 生产与兼容

- **部署前检查**：维护者本机的 Alpha 58 CLI 在服务端切换前后，都能恢复升级验收人物和目标房间，42 条未确认投递均可读取。公网 API 切换前报告 `0.1.0-alpha.58`，之后报告 `0.1.0-alpha.59`。
- **预检**：可用磁盘约 34.0 GB，健康与联邦检查通过；`02:15Z` 的定时备份跑完以后再切换，避开备份锁。
- **预拉镜像**：三个候选镜像按摘要预先拉进本地缓存，共约 17 秒；新备份要用的 `rclone/rclone:1.75.1` 也提前拉好了。
- **服务端部署**：生产源码从 `e3c5c24` 切到 `31d2fc1`，部署前备份 `20261002T021815795863Z-e18b39de` 通过校验（第一次用 rclone 做对象备份），随后切换到兼容服务器（`02:20:01Z`），四份部署与晋级记录齐全。
- **备份与恢复演练**：同一份备份的隔离恢复演练达到了 PITR 目标，用时 18.21 秒，三个数据库都核对过。演练后生产健康检查通过。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:b35cc245ca8fb45395f766c2db06c36f008cc9ed9348dabf20d1d19ccd114519` |
| identity | `sha256:63de456d0ea4ab1955f3c20d50e6781e0d83e512b5babc4247bb9bbbf0441680` |
| web | `sha256:0ed570447d8001f35aa6402edb4d0d5d077a6900d99a636287cd0a4486acf98b` |

网页部署（约 `02:36Z`）之后：

- 公网 API 与网页运行时清单均报告 Alpha 59，两个桌面来源都被接受；
- 生产观察确认健康与联邦检查通过，签名镜像仍在运行，各服务健康；
- 两个下载入口都指向 Alpha 59 的安装包，只改了下载地址，容器未变；
- 网络 Agent 生产冒烟通过：接入说明（`text/plain`）、创建、自查、读取（带回 20 条上下文）、发送都成功，停用后请求被拒（401）；
- 服务器上的旧候选目录只留最近 3 份（Alpha 57–59），这次删掉 Alpha 56，腾出约 400 MB。

## 实机验收

- **升级**：本机已安装的 Alpha 58 原地升级到候选，升级验收记录完整。升级结束了维护者正在运行的 MCP 和命令行 Agent，桌面端随后自己把人物接回房间。
- **新设备**：复用长期验收设备（Alpha 58 那台，`agent-room.alpha58.acceptance.fresh-device`），Bridge 用保存的授权直接就绪。
- **持续接收**：加入、登记真实 Claude Code 宿主任务、授权、绑定、`doctor`、两轮真实回复、空闲、接管、交回都通过，三份验收报告汇总进候选目录。
- 清理：撤销授权、退出验收房间、停掉隔离的 Bridge、关闭调试端口，桌面端恢复正常，`agent-room doctor` 报告 `ready`。
