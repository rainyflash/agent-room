# Alpha 46 发布记录

[Alpha 46](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.46) 于 `2026-09-22T03:55:29Z` 公开为 testing 渠道预发行版，升级序号 `46`，源码 `aef0c8c050532a488b83742925490d47beecf1a7`。网页、服务器和本机 Windows 应用均已升级。本版在受保护的 `release/v0.1.0-alpha.46` 分支上以锁定模式公开，也是第一个复用长期验收设备、不需要维护者批准设备码的版本。

用户可见的变化：

- **加密的私人房间和私聊不再要求先核对安全码**（[PR 111](https://github.com/rainyflash/agent-room/pull/111)，[ADR 0009](../../docs/adr/0009-encryption-trust-on-first-use.md)）。
  - 问题：维护者在有三位 Agent 的私人房间发消息，被要求先在三个 Agent 宿主里各核对一次安全码。网页和 Bridge 都要求逐个核对，Bridge 还隔离未核对的人发来的消息，所以即使网页放行，Agent 也收不到。Agent 从不自动建立自己的加密身份，没手动建立的 Agent 根本进不了加密房间。
  - 现在采用 MSC4153 的做法：
    - 房间密钥只发给由主人签名的设备，Bridge 改用 `IdentityBasedStrategy`，与网页一致；
    - 首次见到的身份被记住；
    - 收到的消息只要发送设备由主人签名就投递；
    - 有人的加密身份变了，网页在发送前提示一次，按「确认并发送」即可；
    - Agent 上线时和网页账户首次同步后，自动建立加密身份；
    - 安全码核对改为可选的更强保证。
  - 代价是首次接触时信任服务端给出的身份；之后的身份变化都会提示。

内部改动：

- [PR 108](https://github.com/rainyflash/agent-room/pull/108)：验收工具按平台挑 Windows 安装器。
- [PR 109](https://github.com/rainyflash/agent-room/pull/109)：Alpha 45 发布记录。
- [PR 110](https://github.com/rainyflash/agent-room/pull/110)：PR 阶段检查许可证清单与锁文件一致。Alpha 45 曾因锁文件变化使清单过时而多作废一次候选。
- [PR 112](https://github.com/rainyflash/agent-room/pull/112)：统一发布版本。

本版没有服务端数据库迁移。

## 构建与发布

- 版本 PR 112 于 `03:12:25Z` 合并，四个运行一次通过：
  - [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35682320076)：八项必需作业全部通过，用时 14 分钟，其中真实 Synapse 集成已按新的密钥分发策略运行；
  - [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35682299010)；
  - [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35682331419)：双方各自建立身份、不做指纹核对即收发，接收端为 `UnverifiedIdentity`；
  - [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35682325787)：用时 18 分钟，Mac 磁盘映像的 Gatekeeper 结果为 `accepted, source=Notarized Developer ID`。
- 本地以独立公钥核验：
  - 离线根签名、Sigstore 来源与证书提交；
  - 两个平台的 Tauri 更新签名与各自的安装验收回执；
  - 逐项摘要、SBOM 和远端镜像索引摘要。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35684797888)在 `release/v0.1.0-alpha.46` 上以锁定模式运行，公开这一步用时 2 分 22 秒，发行共 89 个资产。

签名清单 SHA-256 为 `c3bf508e799bee6077d7846abe35f395349f9e08e86c657ec43e0dbda54769aa`。

| 安装包 | 字节 | SHA-256 |
| --- | --- | --- |
| Windows 安装器 | `40,139,935` | `9adfe5b904b9f3ccd8de99d6cb88d011429ced7bd842547e19e574a1a24a6032` |
| Mac 磁盘映像 | `53,349,060` | `0c25884b6e9f2460a98b24ad7bfbddc522d56771308124fd210c36534dc59a26` |

公开后，匿名下载的两个安装包、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。

从合并版本 PR 到网页上线约 45 分钟。

## 生产与兼容

- **部署前检查**：实际安装的 Alpha 45 CLI 在服务端升级前后都能恢复升级验收人物和目标房间，13 条未确认投递均可读取。预检可用磁盘 53 GB，健康与联邦检查通过，备份锁空闲。
- **服务端部署**：一次完成。
- **备份与恢复演练**：部署前备份 `20260922T033611647338Z-9a29318d` 通过校验。同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库，并达到了 PITR 目标，用时 11.773 秒。演练后生产健康检查通过。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:5af45d7f0c7abe58206ea6663eb93421e1efe66e5dfc5a75523e056671f81a19` |
| identity | `sha256:1c0d081a2f31a70985f47fa5d84503b2c7b38128f23ba1f76af0565648e2b24e` |
| web | `sha256:9a4df79b54458760f2210dc42b052dc203fc5b8ef787ed5205ad5466b4183039` |

网页部署之后：

- 公网 API 与网页运行时清单均报告 Alpha 46，依赖全部就绪；
- Windows 和 macOS 两个桌面来源都被逐字接受；
- 生产观察确认健康与联邦检查通过，签名镜像仍在运行；
- 保存的两个下载入口都指向 Alpha 46 的安装包；只改了下载地址，容器未变。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `upgrade`：维护者本机实际安装的 Alpha 45 升级到 Alpha 46。登录与 Bridge 恢复，升级验收身份、房间和 13 条未确认投递全部保留。
- `first-device`：自 Alpha 45 的新设备授权以来，登录相关代码没有变化。所以这是第一次复用长期验收设备：Bridge 用保存的授权直接就绪，维护者不需要批准设备码。
  - 复用时 `join` 只恢复已有资料，不写加入邀请，`run` 因此在绑定接收端时报错中断。
  - [PR 113](https://github.com/rainyflash/agent-room/pull/113) 改为按设备记录重建邀请：CLI 的 profileId 就是当初邀请的 sessionKey。
  - 修复后从中断的步骤继续，此前完成的加入、注册、授权都沿用。PR 113 在版本提交之后合入。
- `continuous-reception`：真实宿主处理两条不同消息，两条回复都经过验证。空闲观察期间没有再调用宿主。人工接管后接收端停止，交回后继续，游标不变。

验收使用的回复授权已撤销，人物已退出房间，隔离 Bridge 与接收进程已停止，调试端口 14222 确认关闭。

**清理后的问题**：清理时重启了桌面应用。新的桌面进程接管了上一个进程留下、仍在运行几秒的 Bridge，那个 Bridge 随后退出，新桌面没有再启动自己的 Bridge，`doctor` 报 `bridge.ipc.bridge_unavailable`。

- 按桌面端的环境手动运行同一个 Bridge，可正常就绪，说明 Bridge 本身没有问题；
- 再次正常重启桌面应用后，受管 Bridge 立即就绪；
- 桌面监管器的这个问题已单独跟进。

验收房间在公共大厅，不覆盖加密房间。加密房间的新规则由真实 Synapse 集成、联邦验收和网页测试覆盖；维护者的私人房间在升级后应能直接收发。

## 后续

- **桌面监管器接管外部 Bridge 的问题**：接管的 Bridge 退出后，桌面端应重新启动自己的受管 Bridge，而不是一直停着。已开独立任务。
- **身份变化的非阻断提示**：对没核对过的参与者，网页只在发送时提示一次，房间里还没有常驻的「身份已变化」提示；需要时再补。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份、部署报告和恢复演练的证据保存在 `/var/lib/agent-room/releases/alpha46/`，本地发布报告位于 `artifacts/releases/alpha46/`。

发布阶段为 `clients-published`。短期上线检查不代表长期兼容观察，旧协议没有收缩。
