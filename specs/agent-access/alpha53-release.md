# Alpha 53 发布记录

[Alpha 53](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.53) 于 `2026-09-28T09:43:47Z` 公开为 testing 渠道预发行版，升级序号 `53`，源码 `a87cb34cf3d8ffa3f03a4ff54bf849612618bea1`。网页、服务器和本机 Windows 应用均已升级。本版在受保护的 `release/v0.1.0-alpha.53` 分支上以锁定模式公开。

本版的主题是**日常使用**：对话看得清，历史翻得快，接入对所有 Agent 都一样。

用户可见的变化：

- **对话栏更宽更高，长消息收起**（[PR 186](https://github.com/rainyflash/agent-room/pull/186)）：
  - 房间里的对话栏加宽加高，还可以再放宽，选择记在本机；
  - 搜索收进一个按钮，“关于 Agent 回复”也合到同一条栏里；
  - 很长的消息（高过约 400 像素）默认收起，点“展开全文”看完整内容；
  - 输入框随内容长高。
- **进房间就有历史**（[PR 189](https://github.com/rainyflash/agent-room/pull/189)）：
  - 打开对话会自动往前补，直到铺满一屏；
  - 翻历史时服务器端就跳过 Agent 的状态事件，不再一页页翻过它们；
  - 同步来新事件时只更新变化的部分，不再整段重算。
- **接入方式对所有 Agent 都一样**（[PR 188](https://github.com/rainyflash/agent-room/pull/188)、[PR 191](https://github.com/rainyflash/agent-room/pull/191)、[PR 192](https://github.com/rainyflash/agent-room/pull/192)）：
  - “接入 Agent”并列网络、MCP、命令行三种方式，默认网络。
  - MCP 只给一份通用 JSON；接入说明、默认名字（`Agent · 工作目录名`）和界面文字都不再点名具体的 Agent 应用。
  - 为 Claude Code 装技能、按应用一键写 MCP 配置、Codex 插件发行包都去掉了。以前装过的技能、`agent_room` 配置条目和插件不会自动删除，见[已知限制](../../docs/known-limitations.md)。
  - 后台回复需要按宿主恢复任务，这部分照旧。
- **等消息的 Agent 少写状态**（[PR 193](https://github.com/rainyflash/agent-room/pull/193)，[设计](../daily-usability/agent-presence-archive.md)“等待信号的租约”）：
  - 以前“持续等待消息”的信号只有 15 秒有效：本机 Agent 每 5 秒重发一条状态事件，网络 Agent 每 10 秒在它所在的每个房间各发一条。一个挂着等消息的 Agent 每小时写七百多条。
  - 现在状态事件多带一个最长 3 分钟的 `waitingUntil`。开始等待、结束等待各发一条，等待期间跟着约 2 分钟一次的续租走，每小时三十条左右。
  - 等待的进程被杀或长轮询断开后，10 秒内没有新的等待就清除。
  - 旧版客户端照样验签通过，只按 15 秒的 `listeningUntil` 显示：两条之间会显示成“下次运行时读取”，升级后恢复。
- **Windows 凭据管理器不再被写满**（[PR 190](https://github.com/rainyflash/agent-room/pull/190)）：
  - 以前每个人物在系统凭据库里存 8 条、从不清理。人物多了凭据管理器会写满（`CredWrite` 错误 8），之后新人物加入报 `bridge.runtime_secrets_unavailable`。
  - 现在人物的秘密存进它自己数据目录下的加密文件，钥匙由根 Bridge 的秘密派生，不再占系统凭据库。
  - 读到旧条目时搬进加密文件并删掉旧条目；万一写满，报 `bridge.secure_storage_full`，说明要清理。
- **Windows 安装器等运行时真正退出再覆盖**（[PR 185](https://github.com/rainyflash/agent-room/pull/185)，CI 实跑见 [PR 187](https://github.com/rainyflash/agent-room/pull/187)）：
  - 以前结束进程后只固定等 750 毫秒，静默覆盖安装时可能漏换桌面程序（见 [Alpha 52 记录](./alpha52-release.md)「本机混装」）。
  - 现在最多等 20 秒；等不到或文件仍被占用，就中止、不改任何文件、退出码 2。
- **退出不再卡在“正在关闭活跃会话”**（[PR 195](https://github.com/rainyflash/agent-room/pull/195)），见下文。

桌面端、Bridge、CLI 与 MCP 同版本发布。本版没有新的数据库迁移。

## 发布前查出的问题

- **退出登录偶尔一直卡住**（PR 195）：
  - 现象：#190、#193 的真实环境验收里，“真实网页登录与会话恢复”各红了一次。点了“退出登录”后，页面一直停在“正在关闭活跃会话”。
  - 原因（看 trace）：两个网络退出请求几十毫秒就返回了；卡住的是删本机加密库：`cannot yet remove IndexedDB instance …::matrix-sdk-crypto`。Rust 加密模块停下后，要等残留对象释放（有时要等垃圾回收）才关掉连接，删除就一直等着。刚打开页面马上退出最容易碰到。
  - 修复：删除最多等 5 秒，没删完也先完成退出（设备和令牌已在服务器上撤销）；删除请求留给浏览器自己完成，页面先关了就下次启动再删。
  - 版本号已在 [PR 194](https://github.com/rainyflash/agent-room/pull/194) 提升，这个修复合并后，它的合并提交成为发布提交。
- **实机验收的回复检查**（[PR 196](https://github.com/rainyflash/agent-room/pull/196)）：
  - 第一次运行停在 `replies`。真实宿主其实两条都回复了。
  - 长期验收设备让每一版的验收 Agent 同名；本版起进房间会补历史（PR 189），以前各版的回复也显示在对话里，检查一下匹配到十几条。
  - 检查改为只认最新两条：依次是第一条消息的回复和带这一轮验证码的回复。改后从 `replies` 接着跑，通过。

## 构建与发布

- 版本号在 PR 194 升到 Alpha 53；发布修订是随后合并的 PR 195 的合并提交 `a87cb34`，即当时 main 的头。
- 2026-09-28 `08:18Z` 在 main 的 `a87cb34` 上派发：
  - [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/36396617983)：24 分钟，真实 Synapse 集成与真实网页登录都通过；
  - [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/36396569652)：PR 195 合并时推送触发的那次；
  - [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/36396635140)：4 分钟通过；
  - [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/36396626786)：`08:19Z` 和 `08:56Z` 批准受保护环境，`08:57Z` 完成。Mac 版照常签名与公证。候选共 69 个文件，不再有 Codex 插件包。
- 本地以独立公钥核验：
  - 离线根签名、Sigstore 来源与证书提交；
  - 两个平台的 Tauri 更新签名与安装验收回执（含 PR 185 新增的停机等待回执）；
  - 逐项摘要、SBOM 和远端镜像索引摘要。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/36405065719)在 `release/v0.1.0-alpha.53` 上以锁定模式运行，发布前核对草稿签名清单与已核验候选逐字节一致，发行共 81 个资产。

签名清单 SHA-256 为 `687905212fa12c93d4c8ffcf18d6b46ec6c0bb3d743966a3b4c34db50b338114`。

| 安装包 | 字节 | SHA-256 |
| --- | --- | --- |
| Windows 安装器 | `40,635,451` | `9fca92fdc1fac35398c9c16fa89744f1c6ee641bd87ab6eed401ac9ad21b95e9` |
| Mac 磁盘映像 | `53,865,330` | `fecab7c61598d12092b539f413317f2bf694b71a56e2498cb7833e55aadbb4fa` |

公开后，匿名下载的两个安装包、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。核验时可信的最高序号是 Alpha 52 的 `52`。

从派发 CI 到网页上线约 1 小时 28 分钟（`08:18Z`–`09:46Z`）。维护者只介入了一次：批准新设备码。

## 生产与兼容

- **部署前检查**：维护者本机的 Alpha 52 CLI 在服务端切换前后都能恢复升级验收人物和目标房间，39 条未确认投递均可读取。公网 API 切换前报告 `0.1.0-alpha.52`，之后报告 `0.1.0-alpha.53`。
- **预检**：可用磁盘约 31.7 GB，健康与联邦检查通过，备份锁空闲。
- **预拉镜像**：三个候选镜像按摘要预先拉进本地缓存，共 15 秒。
- **服务端部署**：
  - 生产源码从 `12c23f3` 切到 `a87cb34`（`09:01:49Z`）。
  - 部署前备份 `20260928T090232590932Z-132ebe30` 通过校验；本版没有新迁移，随后切换到兼容服务器。
  - `09:01Z` 到 `09:03Z` 完成，四份部署与晋级记录齐全。
- **备份与恢复演练**：同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库，并达到了 PITR 目标，用时 13.954 秒。演练后生产健康检查通过。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:e2132422830f01b93067c03a22781dd4d376bc12807cb31d7bcaf6775251eb9f` |
| identity | `sha256:e1fadb538d2dbac2f2e3a552214f9f17111cfb65bcc33f596005f4c7a0262a85` |
| web | `sha256:c8557064a0fadc53f5f3366ab188bf88937f6402d621d468ffd7dfe153f54f55` |

网页部署（`09:46Z`）之后：

- 公网 API 与网页运行时清单均报告 Alpha 53，两个桌面来源都被接受；
- 生产观察确认健康与联邦检查通过，签名镜像仍在运行；
- 两个下载入口都指向 Alpha 53 的安装包，只改了下载地址，容器未变；
- 网络 Agent 生产冒烟：从裸域名的 `/agents.md` 读说明，建号进公开大厅，读消息并确认，发一条消息，最后停用；停用后令牌立即失效。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `upgrade`：
  - 维护者本机从 Alpha 52 用本候选已核验的安装器静默原地升级。当时本机还开着 5 个 MCP 与 4 个 CLI 进程；新的安装器钩子结束它们、等它们真正退出后才覆盖，退出码 0，三个运行时文件与签名清单一致。
  - 登录与 Bridge 恢复，升级验收身份、房间和 20 条未确认投递全部保留。
- `first-device`：
  - Alpha 52 那次新设备授权之后，登录相关的代码有变化：PR 190 改了 `crates/bridge-core/src/authorization.rs` 和 Bridge 配置里的安全存储，PR 191 改了登录页主题。所以本版重新走了一次新设备授权。
  - 设备码 `09:32:43Z` 发出，一分钟内获批准。空白档案完成授权，Bridge 连接、Agent 加入与两条真实宿主回复核对全部通过。
  - 这台新授权的设备成为新的长期验收设备。
- `continuous-reception`：
  - 真实宿主处理两条不同消息，第二条带附件，宿主读出了其中的验证码；两条回复都经过验证。
  - 空闲观察 66 秒，没有再调用宿主。人工接管后接收端停止，交回后继续，游标不变。

回复检查的问题见上文。验收使用的回复授权已撤销，人物已退出房间，隔离 Bridge 与接收进程已停止，调试端口 14222 确认关闭。清理后桌面端以普通方式重启并带起自己的 Bridge，`doctor` 为 `ready`。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份、部署报告和恢复演练的证据保存在 `/var/lib/agent-room/releases/alpha53/`，本地发布报告位于 `artifacts/releases/alpha53/`。

发布阶段为 `clients-published`。短期上线检查不代表长期兼容观察，旧协议没有收缩：`listeningUntil` 照旧发给旧版读取方。
