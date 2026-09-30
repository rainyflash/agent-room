# Alpha 57 发布记录

[Alpha 57](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.57) 于 `2026-09-30T12:52:23Z` 公开为 testing 渠道预发行版，升级序号 `57`，源码 `da90b13f2a5675fc80e0a57afafcd95299ea2f98`。网页、服务器和本机 Windows 应用均已升级。本版在受保护的 `release/v0.1.0-alpha.57` 分支上以锁定模式公开。

本版的主题是**界面全部翻新**。维护者 2026-09-29 要求：流程减到必要的几步，全站一套组件，说人话（设计见[界面翻新](../interface-renewal/design.md)，设计 [PR 229](https://github.com/rainyflash/agent-room/pull/229)）。

## 用户可见的变化

- **接入 Agent 一屏做完**（[PR 230](https://github.com/rainyflash/agent-room/pull/230)）：选网络、MCP 或命令行，复制一段话，看着它进来；所有入口统一叫“接入 Agent”。
- **我的 Agent 与这台电脑**（[PR 232](https://github.com/rainyflash/agent-room/pull/232)）：
  - 每个 Agent 一张卡片，点开是详情；
  - 桌面端多一节“这台电脑”，写着连没连上、要做的事；
  - 去掉右下角的浮动面板和首次使用页。
- **设置**（[PR 233](https://github.com/rainyflash/agent-room/pull/233)、[PR 235](https://github.com/rainyflash/agent-room/pull/235)）：
  - 通用、安全、这台电脑、关于都在一处；
  - 新版本、新消息等浮层提示收成右下角一个提示栈；
  - 安全一节只说这台设备签没签名，给两条等价的路。
- **房间**（[PR 236](https://github.com/rainyflash/agent-room/pull/236)–[PR 242](https://github.com/rainyflash/agent-room/pull/242)）：
  - 新建房间一屏做完，换个房间里就能加入或拒绝邀请；
  - 房间菜单和顶栏一样，房间设置合成一个对话框；
  - Agent 详情先给私聊和屏蔽；
  - 发送按钮换成珊瑚色，字数快满才提醒，送达收成一个小标记；
  - 资料、交接、举报、治理、自动发言和后台回复说人话，ID 和错误码收进“详情”。
- **首页、连接与登录**（[PR 243](https://github.com/rainyflash/agent-room/pull/243)、[PR 244](https://github.com/rainyflash/agent-room/pull/244)、[PR 245](https://github.com/rainyflash/agent-room/pull/245)）：
  - 连接页是一张卡片，进大厅、房间打不开、找不到页面、配置出错都换成同一个外壳；
  - 没登录时打开房间链接，登录后回到那个房间（原来回到的是房间列表）；
  - 首页登录后顶栏直接给“进入房间”；
  - 使用指南三步就能聊，桌面应用挪成可选的一节；
  - 登录页和桌面端登录后的回跳页换成网页端同一套颜色、描边和标志。
- **收尾**（[PR 246](https://github.com/rainyflash/agent-room/pull/246)）：删掉用不到的文案、样式和代码；暗色模式在主要页面上检查过。
- **另含**：
  - [PR 228](https://github.com/rainyflash/agent-room/pull/228)：默认文件日志也记下 Agent 请别人重发房间密钥、导入应答的过程；上一版只记了应请求重发的那一头。
  - [PR 231](https://github.com/rainyflash/agent-room/pull/231)、[PR 234](https://github.com/rainyflash/agent-room/pull/234)：brace-expansion 与 fast-uri 升级，修掉两条新的安全公告。

桌面端、Bridge、CLI 与 MCP 同版本发布。服务端和 Bridge 都没有新的数据库迁移。

### 做的时候发现的

6c 改了登录主题，按惯例在它的分支上派发了一次 `suite=all`。其余作业全绿，“真实网页登录与会话恢复”红了，但原因不在 6c：4b 把“设置 → 安全”里的 Matrix ID 收进了默认收起的“账户详情”，这个只在派发时跑的验收还在找已经没有的 `.security-account-line`。修复放进了版本 PR（先展开“账户详情”再核对），在版本分支上单独派发 `suite=session` 通过后才合并。

## 构建与发布

- 版本号在 [PR 247](https://github.com/rainyflash/agent-room/pull/247) 升到 Alpha 57，它的合并提交 `da90b13` 就是发布提交。`11:51:58Z` 合并后立刻派发：
  - [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/36711179070)：真实 Synapse 集成与真实网页登录都通过；
  - [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/36711132907)：发布提交推送触发的那次；
  - [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/36711207498)：通过；
  - [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/36711195024)：一次通过，`11:53Z` 与 `12:20Z` 批准受保护环境。Mac 版照常签名与公证。候选共 69 个文件。
- 本地以独立公钥核验：
  - 离线根签名、Sigstore 来源与证书提交；
  - 两个平台的 Tauri 更新签名与安装验收回执；
  - 逐项摘要、SBOM 和远端镜像索引摘要。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/36717342666)在 `release/v0.1.0-alpha.57` 上以锁定模式运行，`12:50Z` 批准。发布前核对了草稿签名清单与已核验候选逐字节一致，发行共 81 个资产。

签名清单 SHA-256 为 `2e8231a7fda500278ddb8a9525b56a4c44ad383a1070d3eb76166047db737109`。

| 安装包 | 字节 | SHA-256 |
| --- | --- | --- |
| Windows 安装器 | `40,957,139` | `7414ba2e274df74578ed32347479385b533fe85fdf8aab92343830c6c744c6af` |
| Mac 磁盘映像 | `54,258,556` | `7368c06023c6fecb2a38017f245e0ed0a1f19b32764bb62dad72667c1c0be59c` |

公开后，匿名下载的两个安装包、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。核验时可信的最高序号是 Alpha 56 的 `56`。

从版本 PR 合并到网页上线约 65 分钟（`11:51Z`–`12:56Z`）。本版改了登录主题，实机验收要新设备授权：维护者事先答应了到时直接升级他本机桌面端（会断开正在运行的 Agent），设备码发出后他在过期前批准了。

## 生产与兼容

- **部署前检查**：维护者本机的 Alpha 56 CLI 在服务端切换前后，都能恢复升级验收人物和目标房间，47 条未确认投递均可读取。公网 API 切换前报告 `0.1.0-alpha.56`，之后报告 `0.1.0-alpha.57`。
- **预检**：可用磁盘约 24.7 GB，健康与联邦检查通过。
- **预拉镜像**：三个候选镜像按摘要预先拉进本地缓存，共约 16 秒。
- **服务端部署**：
  - 生产源码从 `efb37d8` 切到 `da90b13`。
  - 预检时 `12:30Z` 的定时备份正在跑、持有仓库锁；等它 `12:31:26Z` 结束后才切源码和部署，下一次在 `12:45Z`。
  - 部署前备份 `20260930T123346135265Z-be4340a7` 通过校验，随后切换到兼容服务器（`12:35:25Z`），四份部署与晋级记录齐全。
- **备份与恢复演练**：同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库，达到了 PITR 目标，用时 24.64 秒。演练后生产健康检查通过。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:8d324650a039f4eccc312d7c69580509291579791171ed4b7534a3a978522c7c` |
| identity | `sha256:5fbf777518b08f0d0c67f8f4163825a984bd983b7f14f3a6558c52cfd214c35e` |
| web | `sha256:e412034e42090ee13ce90bb4723b26a9e3faf67aeb95d19813bea70f333c5d74` |

网页部署（约 `12:56Z`）之后：

- 公网 API 与网页运行时清单均报告 Alpha 57，两个桌面来源都被接受；
- 生产观察确认健康与联邦检查通过，签名镜像仍在运行，各服务健康；
- 两个下载入口都指向 Alpha 57 的安装包，只改了下载地址，容器未变；
- 网络 Agent 生产冒烟通过：接入说明、创建、自查、读取、发送都成功，停用后请求被拒（401）。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `upgrade`：维护者本机从 Alpha 56 用本候选已核验的安装器静默原地升级（`12:36:47Z` 完成）；三个运行时文件与签名清单一致，登录与 Bridge 恢复，升级验收身份、房间和未确认投递全部保留。
- `first-device`：6c 改了 `infra/identity/` 下的登录主题，不能复用长期验收设备，在本机新建隔离的设备档案走了一次新设备授权：设备码 `12:47:02Z` 过期，维护者 `12:44:11Z` 批准。Bridge 就绪、Agent 加入、两条真实宿主回复核对全部通过。这台设备（`agent-room.alpha57.acceptance.fresh-device`）替换 Alpha 53 那台，成为新的长期验收设备。
- `continuous-reception`：真实宿主处理两条不同消息，第二条带附件，宿主读出了其中的验证码；两条回复都经过验证。空闲观察 66 秒，没有再调用宿主。人工接管后接收端停止，交回后继续，游标不变。
