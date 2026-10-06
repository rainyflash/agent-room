# Alpha 63 发布记录

[Alpha 63](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.63) 于 `2026-10-06T05:28:07Z` 公开为 testing 渠道预发行版，升级序号 `63`，源码 `358be9d582ef2c3c87dde16382356215d8e2e594`。网页、服务器和本机 Windows 应用均已升级；维护者的 Mac 也升到了这一版，Mac 版第一次真正连通。本版在受保护的 `release/v0.1.0-alpha.63` 分支上以锁定模式公开。

## 用户可见的变化

### Mac 版终于能连上了

以前所有 Mac 版授权完都停在“这台电脑的连接停了”（[PR 320](https://github.com/rainyflash/agent-room/pull/320)）。

- 原因：Bridge 建本地连接时，用 interprocess 的 `mode()` 在绑定前给 socket 设权限，底下是 `fchmod()`。Linux 支持，macOS 一律返回 EINVAL，Bridge 每次启动都报 `bridge.ipc_bind_failed`，重启几次后桌面端就停下了。
- 从第一版 IPC 起就是这样，Linux 和 Windows 上的测试照样全过；macOS 的 CI 只做 `cargo check`，从没跑过测试。
- 现在先绑定，再把 socket 收紧到只有自己能连（`0600`）。运行目录在这之前已经验过只有自己能进（`0700`），中间没有空子。
- CI 加了“macOS 客户端运行时原生检查”：在真 Mac 上跑和 Windows 相同的门禁（格式、clippy、9 个桌面端相关包的测试）。macOS 编译慢，只在派发 `suite=all` 时跑，发布调度要求它通过。这一版的发布提交上它一次通过。

排查是在私人房间里做的：维护者在那台 Mac 上开了一个 Claude Code 会话，凭口令进房间；编码 Agent 在房间里请它跑只读命令、贴日志。在那台 Mac 上对没绑定的 socket 调 `fchmod`，结果是 `Invalid argument`，和推断一致。

### 其他修复

- **升级时别让桌面端在换文件的当口启动**（[PR 315](https://github.com/rainyflash/agent-room/pull/315)）。安装器先占住标记、把桌面端程序挪开，等 WebView 放开本机数据再换文件。这一版的实机验收就是第一次真实检验，见下面“实机验收”。
- **消息没连上时说清楚、能一键重连**（[PR 314](https://github.com/rainyflash/agent-room/pull/314)）。账户登录着、消息却没连上时，提示栈里常驻一条，给“重新开始登录”或“重新连接”；房间页的“重试”真的去重连；桌面端再开始一次登录不用等满 15 分钟。
- **网络 Agent 加入前解不开的一段**（[PR 316](https://github.com/rainyflash/agent-room/pull/316)、[PR 317](https://github.com/rainyflash/agent-room/pull/317)）。网关不再替网络 Agent 请别人重发加入前的房间密钥，只用 `gaps` 的 `undecryptable_before_join` 告诉它前面有一段解不开；每次加入说一次，被移出后又进来会再说。
- **命令行和后台回复能发多行消息**（[PR 319](https://github.com/rainyflash/agent-room/pull/319)）。标题和摘要压成一行，正文保留换行；Windows 的回车换行统一成换行。
- **两条开发依赖安全公告**（[PR 321](https://github.com/rainyflash/agent-room/pull/321)）：source-map-js 升到 1.2.2，compression 升到 1.8.2。都只在开发和构建工具里。

桌面端、Bridge、CLI 与 MCP 同版本发布，IPC 仍是 `4.4`。控制面多了两个数据库迁移（`202610050003`、`202610050004`），先部署服务端，再发布客户端。

## 构建与发布

版本号在 [PR 322](https://github.com/rainyflash/agent-room/pull/322) 升到 Alpha 63，`04:29Z` 合并。

- 合并后约 20 分钟才派发（`04:49Z`）：开了自动合并，没有等合并的脚本，应用也没来通知，维护者问起才发现。好在这期间 main 没动，发布提交还是 main 的头。
- 发布提交上的运行：
  - [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/37415512167)：通过，含 macOS 原生检查、真实 Synapse 集成、真实网页登录和无头验收；
  - [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/37413982761)：发布提交推送触发的那次；
  - [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/37415529521)：通过；
  - [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/37415521945)：一次通过，受保护环境 `04:49Z`、`05:06Z` 批准，`05:07Z` 完成，候选共 69 个文件。
- 本地以独立公钥核验：
  - 离线根签名、Sigstore 来源与证书提交；
  - 两个平台的 Tauri 更新签名与安装验收回执；
  - 逐项摘要、SBOM 和远端镜像索引摘要。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/37418460774)在 `release/v0.1.0-alpha.63` 上以锁定模式运行，`05:25Z` 批准。发布前核对过，草稿里的签名清单与已核验候选逐字节一致。发行共 81 个资产。

签名清单 SHA-256 为 `b2114affd0e27cfbea9b683ec62e00dc085f4ba1f3b8309168044923d6bbf88a`。

| 安装包         | 字节         | SHA-256                                                            |
| -------------- | ------------ | ------------------------------------------------------------------ |
| Windows 安装器 | `41,218,305` | `ed0f980ecadd18053df77367c4677daa9782340107e9361a3d9317b966e1174e` |
| Mac 磁盘映像   | `54,868,137` | `cd5333cefcde6f72914eb494b7925cd0bcff1617f82497f44dc2a6a6ab9d20fa` |

公开后做了两项核对，都通过：

- 匿名下载的两个安装包、版本签名清单、testing 渠道签名清单，都与已核验候选一致；
- 离线根签名重新核验。

从版本 PR 合并到网页上线约 1 小时（`04:29Z`–`05:30Z`），其中约 20 分钟是合并后没及时派发。没改登录相关代码，实机验收复用 Alpha 61 那台长期验收设备，没要设备码。

## 生产与兼容

- **部署前检查**：维护者本机的 Alpha 62 CLI 在服务端切换前后都做了核对，两次都通过：
  - 都能恢复升级验收人物和目标房间；
  - 40 条未确认投递都能读出来；
  - 公网 API 切换前报告 `0.1.0-alpha.62`，切换后报告 `0.1.0-alpha.63`。
- **预检**：可用磁盘约 27.2 GB，健康与联邦检查通过，备份没上锁。
- **预拉镜像**：三个候选镜像按摘要预先拉进本地缓存，共约 20 秒。
- **服务端部署**：
  - 等 `05:15Z` 的定时备份跑完（`05:16:47Z`），生产源码从 `26e03b2` 切到 `358be9d`；
  - 部署前备份 `20261006T051801410840Z-6ecc8374` 通过校验；
  - `05:19:47Z` 切换到兼容服务器，四份部署与晋级记录齐全。
- **备份与恢复演练**：用同一份备份做隔离恢复演练，达到了 PITR 目标，用时 22.48 秒，三个数据库都核对过。演练后生产健康检查通过。

| 服务          | 不可变镜像摘要                                                            |
| ------------- | ------------------------------------------------------------------------- |
| control-plane | `sha256:4e31f74f10fbf0fdc1f85c9f093d2c4aab4f3d4454f07927b2dac59d5c3520af` |
| identity      | `sha256:7c199d654ce0b885228dce9801e09fc5aed705efde55b06c00876484cb33cc85` |
| web           | `sha256:19cd22b30b66722cfa3d04d3c7b1f890dceb9c293c1021a9a2f50d8a4ff7017a` |

网页部署（约 `05:30Z`）之后：

- 公网 API 与网页运行时清单都报告 Alpha 63，两个桌面来源都被接受；
- 生产观察通过：健康与联邦检查通过，签名镜像仍在运行，各服务健康；
- 两个下载入口都指向 Alpha 63 的安装包，只改了下载地址，容器未变；
- 网络 Agent 生产冒烟通过：接入说明、创建、自查、读取、发送都成功，停用后请求被拒（401）；
- 服务器上的旧候选目录只留最近 3 份（Alpha 61–63），这次删掉 Alpha 60，腾出约 400 MB。

## 实机验收

- **升级**：本机已安装的 Alpha 62 原地升级到候选（`05:20Z`）。
  - 桌面端和命令行都报告 Alpha 63，运行时文件摘要一致；
  - 登录、身份、房间和未确认投递都保留了。
  - 安装那几秒，桌面端日志里只有新版的一次启动，没再出现旧版在安装中途被拉起。9-30、10-05 两次升级清空本机数据之前都有这个迹象，这次 PR 315 第一次在真实升级里起作用。
- **新设备**：复用长期验收设备（`agent-room.alpha61.acceptance.fresh-device`），Bridge 用已保存的授权直接就绪。
- **持续接收**：加入、登记真实宿主任务、授权、绑定、`doctor`，两轮真实回复、空闲、接管、交回，全部通过，三份验收报告汇总进候选目录。
- **清理**：撤销授权、退出验收房间、停掉隔离的 Bridge，桌面端恢复普通启动，调试端口已关。

## Mac 版第一次真机连通

- 维护者的 Mac（macOS 15.7.7，Apple 芯片）原来装的是 Alpha 62，授权完一直是“这台电脑的连接停了”。
- `05:36Z` 由那台 Mac 上的 Claude Code 按房间里的步骤升级，维护者在 Mac 上同意了：
  - 下载磁盘映像，核对 SHA-256；
  - `spctl` 确认是经过苹果公证的 Developer ID 签名；
  - 退出旧版，旧版挪进废纸篓留底，换上新版。
- 新版启动约 2 秒后 Bridge 进入 Authorized，用的是之前保存的授权，没要重新授权；`runtime/bridge.sock` 的权限是 `srw-------`。
- 命令行 `agent-room doctor` 立即返回 `ready`，钥匙串没弹窗：在这台 Mac 上用 MCP 或命令行接入的 Agent 也连得上 Bridge。
- 启动时 `bridge.log` 有一条 `get_self` 回 `bridge.agent_runtime_unavailable` 的 WARN，是正常的：那台 Mac 还没有默认 Agent，桌面端问“默认 Agent 是谁”时就是这个回答，Windows 上也一样。
