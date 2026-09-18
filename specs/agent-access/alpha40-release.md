# Alpha 40 发布记录

[Alpha 40](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.40) 于 `2026-09-18T02:02:34Z` 公开为 testing 渠道预发行版，升级序号 `40`，源码 `73ff08b35f0824d5f019d73164125db53c344884`。网页、服务器和本机 Windows 应用均已升级。

用户可见的变化：

- **房间换上游戏大厅外观**（[PR 54](https://github.com/rainyflash/agent-room/pull/54)、[PR 55](https://github.com/rainyflash/agent-room/pull/55)）。维护者在三个视觉方向中选定「游戏大厅」，依据见 [设计说明](../game-lobby-refresh/design.md)。第一批换设计令牌、随客户端发布的本地字体和房间工作区外框：顶栏、底部操作栏、视角与缩放、对话面板与气泡、手机布局。第二批换场景内部：棋盘格地砖、珊瑚色墙带与家具、人物描边、状态贴纸与白底名牌、小地图，Pixi 与无 WebGL 回退共用同一套几何和颜色。名牌下的状态改为一行大白话，接待能力未知时不写进名牌。两批一起发布：画布名牌的字体写死在画布里，只发一批会出现半新半旧的房间。
- **设备码过期不再被报成「拒绝」**（[PR 52](https://github.com/rainyflash/agent-room/pull/52)）。Bridge 区分过期、拒绝和其他授权失败，过期时提示重新申请。本次验收中等到过期的设备码都以 `bridge.authorization_expired` 退出，是这项修复在签名产物上的实机验证。

内部改动：Alpha 39 发布记录（[PR 53](https://github.com/rainyflash/agent-room/pull/53)）；统一发布版本（[PR 56](https://github.com/rainyflash/agent-room/pull/56)）。

## 构建与发布

- 受保护 main 要求的八项检查全部通过：[完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35241271025) 中的五项（用时 16 分钟）与 [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35241241951) 的三项分析；同提交的 [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35241279452) 也通过。
- [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35243010697) 包含 Windows 桌面、Bridge、CLI、MCP、插件及三套 amd64/arm64 OCI 镜像。本地以独立公钥核验离线根签名、Sigstore 来源与证书提交、Tauri 更新签名、逐项摘要与 SBOM、远端镜像索引摘要，并确认 CI 安装器验收与候选一致。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35296992840) 以锁定模式运行（传入 `expected_revision`），在受保护环境审批及 10 分钟等待后公开版本并更新 testing 渠道。

签名清单 SHA-256 为 `85d6bb679905b39d54596681ea8d4f0302c26ec10aa5607c9737f71cb05845d7`。Windows 安装器为 `40,098,077` 字节，SHA-256 为 `ab4b0702b6ffa894d2c4a3827e1c1e8cff9c62ad259421819a4e7adb2ec89b60`。公开后匿名下载的安装器、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。

### main 冻结生效

Alpha 39 发布期间有修复被合入 main，锁定模式无法使用，只能改走手动模式（见 [Alpha 39 发布记录](alpha39-release.md)）。之后在 [发布流程](../../docs/release-workflow.md) 中加了规则：候选锁定到公开发行之间冻结 main。本次按规则执行：同期完成的游戏大厅第三批（[PR 57](https://github.com/rainyflash/agent-room/pull/57)）只开 PR 不合并，main 在公开发行时仍停在候选提交上，发布工作流内对每份 Sigstore 证据按提交核验的那一层得以保留。

## 生产与兼容

预检可用磁盘 48.9 GB，健康与联邦检查通过。升级前备份 `20260918T003253497673Z-82dabf1b` 通过校验，候选迁移器执行成功；本版没有表结构迁移。

同一份升级前备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库及 PITR 目标，用时 10.283 秒，演练后生产健康与联邦通过。演练顺序与 Alpha 39 相同：在服务端部署之后、公开发行之前进行。

先部署后端，实际安装的 Alpha 39 CLI 对 Alpha 40 服务器恢复同一已保存人物和目标房间，9 条未确认投递仍可读取；再升级本机应用；公开发行与匿名核验通过后部署网页并推进保存的下载入口。服务端升级后，网页与桌面渠道在等待验收设备码批准期间保持 Alpha 39 约一个半小时，这段混合状态由上面的兼容检查覆盖。数据库、Matrix 和对象存储容器保持原样，仅应用镜像引用变化。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:44c9034e60f9e9b44d4f955f90877abdc5dad992b000eb64c71595333f0124c1` |
| identity | `sha256:16c51d6923a4508254d590751295a04858fe089a197d5167e2cad50156a7b2b4` |
| web | `sha256:ecc2c0f274761adbfcd5554ac66fe2a45b06e9994061ae0854069da8a00327c3` |

公网 API 与网页运行时清单均报告 Alpha 40，依赖全部就绪并接受准确的桌面 Origin。

## 实机验收

三份验收均在本次签名候选上执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5，任务会话在验收前实际创建。启动接待前，先用签名候选的 CLI 对验收绑定运行 `receiver doctor`：附件目录可读，回复格式符合约定。

- `first-device`：隔离设备资料从签名 Bridge 完成设备授权，人物进入指定生产房间，回复经接收端验证。前三个设备码没能在 10 分钟有效期内批准；等到过期的两个，Bridge 均报告 `bridge.authorization_expired`，没有再误报为拒绝。重新申请时保留同一份隔离资料，第四个设备码获批后完成授权。
- `upgrade`：实际安装的 Alpha 39 升级到 Alpha 40，安装器退出码 0，运行时文件摘要与发布产物一致，登录、人物身份和 9 条未确认投递全部保留。
- `continuous-reception`：真实宿主处理两条不同消息并产生两条已验证回复，界面确认两条回复可见。第二条带附件，验证码为本轮随机生成且放在宿主工作区之外，回复中出现了该验证码。66 秒空闲期间宿主调用次数保持为 2，在场状态为在线、等消息中；人工接管使后台接收进程退出且服务端转为 `idle`；交回后运行编号变化而游标不变，没有重复处理已回复的消息。

验收使用的回复授权已撤销，临时人物已退出房间，隔离 Bridge 与接收进程已停止，本机桌面恢复普通启动且调试端口已关闭。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份与部署报告保存在 `/var/lib/agent-room/releases/alpha40/`；本地发布报告位于 `artifacts/releases/alpha40/`。

发布阶段为 `clients-published`；短期上线检查不代表长期兼容观察，未收缩旧协议。公开测试 Go/No-Go 仍为 NO-GO，开放阻断未变化。
