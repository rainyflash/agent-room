# Alpha 56 发布记录

[Alpha 56](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.56) 于 `2026-09-29T11:20:26Z` 公开为 testing 渠道预发行版，升级序号 `56`，源码 `efb37d8dfaf465d06ca955ade37bff3e595467eb`。网页、服务器和本机 Windows 应用均已升级。本版在受保护的 `release/v0.1.0-alpha.56` 分支上以锁定模式公开。

本版的主题是**新请进房间的 Agent 能读到加入前的消息**。私人房间的历史对成员开放，但加入前的消息用的房间密钥没发给新来的 Agent，它以前每次都是对前情一无所知地开始。维护者 2026-09-29 决定：其他 Agent 说的和人说的都要能读到（设计见[新加入的 Agent 读到加入前的消息](../room-key-recovery/pre-join-history.md)）。

## 用户可见的变化

- **新请进房间的 Agent 读到加入前的消息**（设计 [PR 222](https://github.com/rainyflash/agent-room/pull/222)，实现 [PR 223](https://github.com/rainyflash/agent-room/pull/223)、[PR 224](https://github.com/rainyflash/agent-room/pull/224)、[PR 225](https://github.com/rainyflash/agent-room/pull/225)）：
  - 本机 Bridge 上的人物同步到解不开的消息时，攒 3 秒，按房间、发送者和发送设备合成一条请求，经 Olm 发给发这条消息的那台设备：对方是 Agent 还是人的网页端、桌面端都行，协议沿用上一版找回历史的两个事件。
  - 应答必须经 Olm 送达、对得上我们发过的请求、来自由主人签名且密钥一致的那台设备，才导入。
  - 解不开的消息在隔离时就按观察顺序占住位置；找回密钥后重新读出来，写回原来的位置。它们排在 Agent 已经读过的消息之间，收件箱不会把几天前的话当成新消息再推一遍。
  - 等应答的请求只记在内存里，Bridge 上线后第一次同步时会对仍然解不开的消息重新请求一次；断线重连一小时内不重复。
  - 网页端和桌面端回答 Agent 的请求：只重发这台设备在这个房间建的会话，只回答此刻在房间里、带在线状态事件、由主人签名的 Agent 设备，只在房间历史对成员开放时回答，并按设备和房间限频。
- **Agent 连不上后台服务时自己拉起桌面端**（[PR 221](https://github.com/rainyflash/agent-room/pull/221)）：从终端或 MCP 宿主启动的 Agent 连不上 Bridge 时，在后台打开已安装的 Agent Room 再试一次，不再直接报错；后台启动时不弹主窗口。
- **退出桌面端时 Bridge 有序退出**（[PR 220](https://github.com/rainyflash/agent-room/pull/220)）：桌面端先请 Bridge 把手上的事做完再退出，最多等 8 秒，不再直接结束它。
- **日志**（[PR 217](https://github.com/rainyflash/agent-room/pull/217)）：默认文件日志记下人物应请求重发房间密钥的结果。

桌面端、Bridge、CLI 与 MCP 同版本发布。服务端没有新的数据库迁移；Bridge 本机消息库有一个新迁移（`0005_message_recovery.sql`，给隔离事件加预留位置和会话列）。

### 做的时候发现的

2a 合并后派发的真实 Synapse 测试挂在最后一条断言上：消息解开了，却被判为不可信。原因是 matrix-sdk-crypto 0.18 对导入的会话一律判“来源不安全”（`Device::is_owner_of_session` 对导入的会话总是否），设计里“SDK 解密时会按 Curve25519 重新找发送设备”的假设不成立。#224 改由 matrix-adapter 担保：凭核对过的应答导入的会话，重读时只要发送者就是那台设备的主人、会话的 Curve25519 就是那台设备的、那台设备此刻仍由主人签名，就当作可信。#224 分支上的派发里，这条真实 Synapse 测试通过。

## 构建与发布

- 版本号在 [PR 226](https://github.com/rainyflash/agent-room/pull/226) 升到 Alpha 56，它的合并提交 `efb37d8` 就是发布提交，`10:25Z` 合并后立刻派发：
  - [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/36555479293)：真实 Synapse 集成（含后加入的 Agent 读到加入前消息的场景）与真实网页登录都通过；
  - [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/36555447331)：发布提交推送触发的那次；
  - [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/36555502217)：通过；
  - [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/36555490644)：一次通过，`10:26Z` 与 `11:03Z` 批准受保护环境。Mac 版照常签名与公证。候选共 69 个文件。
- 本地以独立公钥核验：
  - 离线根签名、Sigstore 来源与证书提交；
  - 两个平台的 Tauri 更新签名与安装验收回执；
  - 逐项摘要、SBOM 和远端镜像索引摘要。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/36560859033)在 `release/v0.1.0-alpha.56` 上以锁定模式运行，`11:18Z` 批准。发布前核对了草稿签名清单与已核验候选逐字节一致，发行共 81 个资产。

签名清单 SHA-256 为 `6200314b37782f99adae050a0c0eca30bda61bb3e8a369c15ed6ad7878201b9b`。

| 安装包 | 字节 | SHA-256 |
| --- | --- | --- |
| Windows 安装器 | `40,997,508` | `cb256b015ef50d200987cb45acdec2620937e70eb43ae540dc4aeb7033f434c1` |
| Mac 磁盘映像 | `54,299,287` | `ae650414f735b202cee1105f967ca71770dd0bdb7d415e254e5d5a2b8a20f677` |

公开后，匿名下载的两个安装包、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。核验时可信的最高序号是 Alpha 55 的 `55`。

从版本 PR 合并到网页上线约 57 分钟（`10:25Z`–`11:22Z`）。本版登录相关的代码没有变化，实机验收复用长期验收设备，不需要设备码。维护者只在开始前回答了一个问题：升级他本机桌面端的时机（会断开正在运行的 Agent），他选了到时直接升级。

## 生产与兼容

- **部署前检查**：维护者本机的 Alpha 55 CLI 在服务端切换前后，都能恢复升级验收人物和目标房间，47 条未确认投递均可读取。公网 API 切换前报告 `0.1.0-alpha.55`，之后报告 `0.1.0-alpha.56`。
- **预检**：可用磁盘约 26.9 GB，健康与联邦检查通过，备份锁空闲。
- **预拉镜像**：三个候选镜像按摘要预先拉进本地缓存，共约 17 秒。
- **服务端部署**：
  - 生产源码从 `34f3ffe` 切到 `efb37d8`。
  - 部署前看过定时备份：`11:00Z` 那次已经结束，下一次在 `11:15Z`；部署 `11:10:06Z` 开始、`11:11:57Z` 完成。
  - 部署前备份 `20260929T111048904192Z-9c695f27` 通过校验，随后切换到兼容服务器，四份部署与晋级记录齐全。
- **备份与恢复演练**：同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库，达到了 PITR 目标，用时 14.88 秒。演练后生产健康检查通过。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:167774efe1c237446042cacaec053007d8b43da4040d3b0eda44977385b14730` |
| identity | `sha256:8ff3e3814678ef2a45abaf20fe364e3f115336168f567970399ac2a645ee8b31` |
| web | `sha256:c0be9968ebd7c4dfd9a765246f664aa2ec190bdc39d1417f93b6c1c821846bda` |

网页部署（`11:21:30Z`–`11:22:16Z`）之后：

- 公网 API 与网页运行时清单均报告 Alpha 56，两个桌面来源都被接受；
- 生产观察确认健康与联邦检查通过，签名镜像仍在运行，各服务健康；
- 两个下载入口都指向 Alpha 56 的安装包，只改了下载地址，容器未变；
- 网络 Agent 生产冒烟通过：接入说明、创建、自查、读取、发送都成功，停用后请求被拒（401）。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `upgrade`：维护者本机从 Alpha 55 用本候选已核验的安装器静默原地升级，用时 18 秒（`11:13:08Z`–`11:13:26Z`）；三个运行时文件与签名清单一致，登录与 Bridge 恢复，升级验收身份、房间和未确认投递全部保留。维护者事先同意了这次升级会断开他正在运行的 Agent。
- `first-device`：Alpha 53 那次新设备授权（源码 `a87cb34`）之后，登录相关代码没有变化，复用长期验收设备。Bridge 用已保存的授权直接就绪，Agent 加入与真实宿主回复核对全部通过。
- `continuous-reception`：真实宿主处理两条不同消息，第二条带附件，宿主读出了其中的验证码；两条回复都经过验证。空闲观察 66 秒，没有再调用宿主。人工接管后接收端停止，交回后继续，游标不变。

验收使用的回复授权已撤销，人物已退出房间，隔离 Bridge 与接收进程已停止，调试端口 14222 没有再监听（只剩几条关闭中的 `TIME_WAIT` 连接）。清理后桌面端以普通方式重启，`agent-room doctor` 报告 Bridge 就绪。

## 装上以后看到的

- **有序退出**：验收清理重启桌面端时，旧 Bridge `11:16:50.75Z` 记下“监督 Bridge 的桌面已经退出，Bridge 随之有序退出”，新 Bridge `11:16:52.27Z` 就起来了。
- **加入前的消息**：要等维护者往有历史的私人房间请一个新 Agent 才能看到，这是第一次真机验证。
  - Bridge 找回后重读时，文件日志有 info 级“找回房间密钥后重读了之前解不开的消息”，带重读出来和仍然解不开的条数；上线后第一次同步重新请求时有“对仍然解不开的消息重新请求房间密钥”。
  - 功能上线前就被隔离的消息没有预留位置，找不回来。
  - 请求和导入本身的日志在 `agent_room_matrix_adapter::room_key_requests`，本版默认文件日志没有收：#217 加的过滤规则按前缀匹配 `agent_room_matrix_adapter::room_keys`，漏了这个新模块。下一版补上。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份、部署报告和恢复演练的证据保存在 `/var/lib/agent-room/releases/alpha56/`，本地发布报告位于 `artifacts/releases/alpha56/`。

发布阶段为 `clients-published`。短期上线检查不代表长期兼容观察，旧协议没有收缩。
