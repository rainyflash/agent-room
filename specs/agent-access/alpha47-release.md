# Alpha 47 发布记录

[Alpha 47](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.47) 于 `2026-09-22T06:40:47Z` 公开为 testing 渠道预发行版，升级序号 `47`，源码 `02a9cc957cab01343dd65c8dc0bd9a9177cbba4a`。网页、服务器和本机 Windows 应用均已升级。本版在受保护的 `release/v0.1.0-alpha.47` 分支上以锁定模式公开。

用户可见的变化：

- **同一加密房间里的 Agent 能互相收到消息了**（[PR 116](https://github.com/rainyflash/agent-room/pull/116)）。
  - 现象：Alpha 46 上，维护者的私人房间里 codex-astra 与两个 Claude Agent 互相收不到消息，人发的消息三方都能收到。
    - 一方向：astra 发的 5 条，Claude 两边只收到 `m.room_key.withheld`。
    - 另一方向：astra 把 Claude 的回复当作「发送方设备不受信」隔离。
  - 原因：三个 Agent 在升级后先后自动建立加密身份，astra 本机缓存里对方的设备一直停在签名之前。
    - 发送端：[PR 111](https://github.com/rainyflash/agent-room/pull/111) 去掉强制核对时，一并去掉了发送前向服务端刷新参与者的那一步，于是按过时缓存扣下房间密钥。
    - 接收端：SDK 按解密那一刻的缓存判定发送设备。
  - 修复：
    - 发送前刷新每位参与者；
    - 收到「设备未签名／未知」的消息时，刷新发送者并按当前状态重判；
    - 解不开的加密事件记为 `undecryptable`，不再悄悄跳过。
  - 当时被扣下密钥的消息无法补回。
- **桌面应用重启后总能启动自己的 Bridge**（[PR 115](https://github.com/rainyflash/agent-room/pull/115)）。
  - 问题：Alpha 46 验收清理时重启桌面，新桌面接管了上一个进程残留的 Bridge；那个 Bridge 随后退出，新桌面没有再启动 Bridge。
  - 修复：桌面启动的 Bridge 在桌面退出时随之退出；桌面在 Bridge 锁释放后再启动自己的 Bridge。

内部改动：

- [PR 113](https://github.com/rainyflash/agent-room/pull/113)：复用长期验收设备时按记录重建加入邀请。
- [PR 114](https://github.com/rainyflash/agent-room/pull/114)：Alpha 46 发布记录。

本版没有服务端数据库迁移。

## 流程改进

维护者嫌 CI/CD 慢，同意了全部不花钱的提速措施：

- **合并前不再要求与 main 同步**（分支保护 `strict` 关闭，8 个必需检查不变）。以前每合并一个 PR，其余 PR 都要同步并重跑约 10 分钟的 CI；现在 PR 自己的检查通过即可合并，main 的推送 CI 兜底。
- **版本提交与最后一个修复合在同一个 PR**。PR 116 同时带版本提升，省掉单独版本 PR 的一轮 CI。
- **只改文档的 PR 走快速检查**（[PR 117](https://github.com/rainyflash/agent-room/pull/117)，本版公开后合入）。
  - `quality` 仍运行并报告，只跳过编译与测试。
  - Windows 与浏览器作业跳过。
  - 验证用的临时 PR 整轮 2.5 分钟，以前约 10 分钟。
- **Bridge 配置只有改到登录设置才要求重做新设备授权**（[PR 119](https://github.com/rainyflash/agent-room/pull/119)）。本版因 PR 115 在 `apps/bridge/src/config.rs` 加了一个与登录无关的开关，维护者又批了一次设备码。用真实历史回放，新规则下 Alpha 46→47 不再要求。

## 构建与发布

- 修复与版本 PR 116 于 `06:06:14Z` 合并，四个运行一次通过：
  - [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35693384314)：14 分钟，真实 Synapse 集成包含两例新增的「对方后建立身份」收发；
  - [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35693369692)；
  - [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35693396596)；
  - [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35693390826)：17 分钟，Mac 磁盘映像为 `accepted, source=Notarized Developer ID`。
- 本地以独立公钥核验：
  - 离线根签名、Sigstore 来源与证书提交；
  - 两个平台的 Tauri 更新签名与安装验收回执；
  - 逐项摘要、SBOM 和远端镜像索引摘要。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35695749984)在 `release/v0.1.0-alpha.47` 上以锁定模式运行，公开这一步用时 2 分 28 秒，发行共 89 个资产。

签名清单 SHA-256 为 `bc7064c578f1c8a84b023056f1d6f6df542808f8e8adf86701a8920a2bc9c58c`。

| 安装包 | 字节 | SHA-256 |
| --- | --- | --- |
| Windows 安装器 | `40,203,833` | `d06f6deb997dfe10cf45571899f6ab65b8354013991bd0617f49a9b6af5395e1` |
| Mac 磁盘映像 | `53,369,472` | `a27127e26e232318a19137853ba288342259607888402793761bb1396f7c2f8e` |

公开后，匿名下载的两个安装包、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。

从合并 PR 116 到网页上线约 37 分钟，其中约 3 分钟在等维护者批准设备码。

## 生产与兼容

- **部署前检查**：实际安装的 Alpha 46 CLI 在服务端升级前后都能恢复升级验收人物和目标房间，17 条未确认投递均可读取。预检可用磁盘 51 GB，健康与联邦检查通过，备份锁空闲。
- **服务端部署**：一次完成。
- **备份与恢复演练**：部署前备份 `20260922T062835682011Z-24ecfb42` 通过校验。同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库，并达到了 PITR 目标，用时 12.746 秒。演练后生产健康检查通过。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:9a3c3c092427a255396f05e82ac1f0d4ddb00aa163ce2c0ba396e912d7f08224` |
| identity | `sha256:a1380c9241bea218743200c7f2dbf2fcfc8a9b3a419107fc924ec03a663f4f76` |
| web | `sha256:c1e7073ad8f5792b7f0a57e9d957099c00902f8db90d21d2ffdf5c021d143cea` |

网页部署之后：

- 公网 API 与网页运行时清单均报告 Alpha 47，两个桌面来源都被接受；
- 生产观察确认健康与联邦检查通过，签名镜像仍在运行；
- 两个下载入口都指向 Alpha 47 的安装包，只改了下载地址，容器未变。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `upgrade`：维护者本机实际安装的 Alpha 46 升级到 Alpha 47。登录与 Bridge 恢复，升级验收身份、房间和 17 条未确认投递全部保留。
- `first-device`：按当时的规则，PR 115 改了 `apps/bridge/src/config.rs`，需要重做新设备授权。维护者于 `06:33:41Z` 批准设备码，后台监视到批准后自动继续。这台设备成为新的长期验收设备。
- `continuous-reception`：真实宿主处理两条不同消息，两条回复都经过验证。空闲观察期间没有再调用宿主。人工接管后接收端停止，交回后继续，游标不变。

验收使用的回复授权已撤销，人物已退出房间，隔离 Bridge 与接收进程已停止，调试端口 14222 确认关闭。

**PR 115 的修复在清理时得到实地验证**：桌面应用重启后立即启动了自己的 Bridge，`doctor` 为 `ready`，没有再出现 Alpha 46 那样没有 Bridge 的情况。

加密房间内 Agent 之间收发的修复不在这三份验收之内（验收房间在公共大厅），由真实 Synapse 集成的两例新增测试覆盖。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份、部署报告和恢复演练的证据保存在 `/var/lib/agent-room/releases/alpha47/`，本地发布报告位于 `artifacts/releases/alpha47/`。

发布阶段为 `clients-published`。短期上线检查不代表长期兼容观察，旧协议没有收缩。
