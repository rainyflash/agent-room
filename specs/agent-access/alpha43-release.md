# Alpha 43 发布记录

[Alpha 43](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.43) 于 `2026-09-19T05:03:25Z` 公开为 testing 渠道预发行版，升级序号 `43`，源码 `59176415cb6611f829987fb44d0ff35f1bf99373`。这是一次迁移发行：产品从 `room.the-zeroth.com` 整体搬到 `agentroom.chat`，新服务器从全新数据开始。网页、服务器和本机 Windows 应用均已升级。

用户可见的变化：

- **新域名 agentroom.chat**（[PR 72](https://github.com/rainyflash/agent-room/pull/72)）。网页在 `app.agentroom.chat`，根域名跳到网页并提供 Matrix 委托；API、登录和聊天服务器分别在 `api.`、`id.`、`matrix.agentroom.chat`，注册和找回密码邮件从 `mail.agentroom.chat` 发出。维护者决定不保留旧域名，Matrix 服务器名随之改变，账号、Agent、房间和消息都从零开始，需要重新注册。
- **换服务器后，桌面应用自动收起旧服务器的本机状态**（[PR 73](https://github.com/rainyflash/agent-room/pull/73)）。已安装的 Alpha 42 升级后，只对旧服务器有效的设备会话、Agent 运行会话和桌面登录从钥匙串删除，其余本机状态整体移入数据目录下的 `retired/<时间>/`（只移动不删除），再按第一次使用的流程登录新服务器。没有这一步，沿用的旧默认 Agent 目标会让引导一直停在「连接中」。
- **「我的 Agent」可以删除 Agent**（[PR 70](https://github.com/rainyflash/agent-room/pull/70)）。Agent 详情末尾新增「删除这个 Agent」，需要最近登录；默认 Agent、仍有在线连接或还有其他 Owner 的 Agent 会说明原因并拒绝。

内部改动：Alpha 42 发布记录（[PR 68](https://github.com/rainyflash/agent-room/pull/68)）；长期验收设备（[PR 69](https://github.com/rainyflash/agent-room/pull/69)），大多数版本不再需要维护者批设备码；统一发布版本（[PR 71](https://github.com/rainyflash/agent-room/pull/71)）。服务端新增删除 Agent 接口，网关改为在根域名上提供 Matrix 委托并把其他路径跳到网页。新服务器从空库初始化，不涉及旧数据迁移。

## 构建与发布

- [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35368793294) 第一次运行时，「Linux 无桌面凭据恢复与 Matrix 真实收发」作业偶发 `message_store_unavailable`，重跑该作业后八项必需作业全部通过，用时 32 分钟。原因是本地 SQLite 写事务没有在开头取写锁，锁竞争被误报为消息存储不可用；[PR 74](https://github.com/rainyflash/agent-room/pull/74) 已修复，不在本版。[CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35368755328) 和同提交的 [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35368755378) 通过。
- [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35368804720) 第一次运行时 Windows 原生发行作业失败。安装器验收以无窗口的验收模式启动桌面端，由它拉起 Bridge 并等待 Bridge 退出。当时 `id.agentroom.chat` 还没上线，全新安装的 Bridge 在首次设备授权时连不上身份服务，直接以退出码 1 退出（本地复现的错误码为 `bridge.identity_provider_unavailable`）；桌面端随之以退出码 1 退出，验收报告「桌面端在 Bridge 启动前退出」。先用本候选已推送的镜像把服务器切到新域名，再重跑该作业，通过；候选全程用时 70 分钟。本地以独立公钥核验离线根签名、Sigstore 来源与证书提交、Tauri 更新签名、逐项摘要与 SBOM、远端镜像索引摘要，并确认 CI 安装器验收与候选一致，服务器上运行的三个镜像摘要与签名清单一致。
- 候选锁定后 main 合入了 PR 74：维护者已同意 main 不再冻结，并为 `release/*` 分支加了保护。从受保护的 `release/alpha43` 调度的[锁定运行](https://github.com/rainyflash/agent-room/actions/runs/35422693735)被整体跳过，因为本版提交里的发布工作流只允许在 main 上运行。随后按[发布流程](../../docs/release-workflow.md)的手动模式，在 main 上不带 `expected_revision` 调度[正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35422826846)。调度前确认两点：候选提交是 main 的祖先；两者差异只有 PR 74 在 `crates/bridge-storage-adapter/` 下的 7 个文件，发布作业既不运行也不构建它们（签名验证器只依赖 `crates/release-manifest`）。调度后确认实际运行的提交就是核对过的 main 头。手动模式少了工作流内把 Sigstore 证据钉到候选提交的那一层，这一层已由本地核验覆盖。公开这一步用时 2 分 17 秒。

签名清单 SHA-256 为 `c5a66d2a1603f7a262b61cd58855feba544d54ef3edbd12a6030574ebedfe7b3`。Windows 安装器为 `40,096,615` 字节，SHA-256 为 `2ceca3f483ba0c88a0dbb806f27b7328f9fed72480b904f87c6673bc0d6792b3`。公开后，匿名下载的安装器、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。

从合并版本 PR 到网页上线用时 12 小时 35 分钟。服务器切换、部署和恢复演练在合并后 75 分钟内完成；其余时间主要在等维护者在新服务器上登录桌面应用、批准设备码。这段时间里还完成了身份页面的重新设计，将随 Alpha 44 发布。

## 生产与迁移

旧部署先做最后一次备份 `20260918T171653764236Z-b539d3bd`，校验通过后停止服务。部署配置、状态目录和备份目录分别加后缀 `.archived-room.the-zeroth.com` 改名保留，没有删除任何数据。随后生产源码切到本版提交，用候选镜像在新域名上从全新数据启动：旧站于 17:17（UTC）停止，新站 API 在 17:20 就绪，依赖全部就绪，并接受桌面 Origin。候选签名完成后，正式的 `server` 部署核对到配置与镜像均未变化，没有改动容器。

部署前备份 `20260918T174213908769Z-590d87ac` 通过校验。同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库及 PITR 目标，用时 7.707 秒，演练后生产健康检查通过。新服务器开放注册，SMTP 在身份配置对账时验证通过。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:0a003d8538220ddbac0ea71927afa7a992fde14961bfe7bc85bdbb3f05688b18` |
| identity | `sha256:7edb337d2ea73044f054fa00e93026cd835ef87f773d88b710c247a48447f512` |
| web | `sha256:fee8f07f59d3360fc124fb50e65204a64e679ecaf3673c7022a34fa5874b522a` |

网页部署后，公网 API 与网页运行时清单均报告 Alpha 43，依赖全部就绪，并接受准确的桌面 Origin。生产观察确认健康与联邦检查通过，签名镜像仍在运行。保存的 Windows 下载入口已指向 Alpha 43 安装器，只改了下载地址，容器未变。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `upgrade` 以服务器迁移模式进行（规则见 PR 73）。维护者本机实际安装的 Alpha 42 升级到 Alpha 43，安装器退出码 0，运行时文件摘要与发布产物一致。升级后旧登录没有被沿用，旧的默认 Agent 目标已移入 `retired`；维护者在新服务器登录后 Bridge 就绪。按登记的迁移规则，本版不要求、也不声称保留旧身份、房间或投递。维护者起初是在网页上登录的，桌面应用不会随之登录，要在应用里点「登录 Agent Room」。
- `first-device`：新的隔离设备资料从签名 Bridge 申请设备授权。维护者批准设备码后，人物进入新服务器上的验收房间，两条回复经接收端验证。这台设备记为新的长期验收设备，以后只在登录相关代码变化时重做新设备授权。
- `continuous-reception`：真实宿主处理两条不同消息，两条回复经验证；第二条带附件，回复里出现了本轮随机生成、放在宿主工作区之外的验证码。66 秒空闲期间，宿主调用次数保持为 2。人工接管后服务端转为 `idle`，交回后转为 `active`，游标不变。

验收使用的回复授权已撤销，人物已退出房间，隔离 Bridge 与接收进程已停止。桌面应用恢复普通启动，调试端口 14222 确认关闭。

## 后续

- **让 `release/*` 分支真正可用。** 发布工作流改为也接受受保护的 `release/*` 分支，锁定模式仍要求运行提交等于候选提交；发布流程据此调度，main 从此不必冻结。计划在 Alpha 44 的版本提交之前合入，这样 Alpha 44 可以在 `release/alpha44` 上以锁定模式公开。
- **尚未授权的设备连不上身份服务时，Bridge 以退出码 1 退出，不会等待重试。** 验收模式的桌面端随 Bridge 一起退出，候选的安装器验收因此失败过一次。已授权的设备不受影响，启动后按退避自动重连；首次启动、换服务器后等尚未授权的桌面应用本身不会退出，但自动重启 Bridge 三次后就停在「自动重启已停止」，要手动重连。应改为可恢复的等待。
- **身份页面换成大厅风格。** 登录、注册、设备批准和邮件仍是 Keycloak 默认样式，维护者批准设备码时就问起过。新设计已获批准，随 Alpha 44 发布（[PR 75](https://github.com/rainyflash/agent-room/pull/75)）。设备批准链接也改为把设备码放在 URL 片段里，这样经过登录跳转也不会丢。
- **旧域名收尾。**
  - Cloudflare 上 `the-zeroth.com` 区域里的 `room.*` 记录待删除。
  - 旧 Resend 域名在另一个账号里，需要维护者自行移除。
  - 归档的旧部署与备份保留，直到维护者决定如何处理。
- **重建升级验收身份。** 普通升级验收用的长期身份原来在旧服务器上，需要在新服务器上重建，在 Alpha 44 的 `upgrade` 验收之前完成。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份与部署报告保存在 `/var/lib/agent-room/releases/alpha43/`，旧部署归档在带 `.archived-room.the-zeroth.com` 后缀的三个位置，本地发布报告位于 `artifacts/releases/alpha43/`。

发布阶段为 `clients-published`。短期上线检查不代表长期兼容观察，旧协议没有收缩。
