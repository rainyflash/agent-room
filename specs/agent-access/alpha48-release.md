# Alpha 48 发布记录

[Alpha 48](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.48) 于 `2026-09-22T11:46:17Z` 公开为 testing 渠道预发行版，升级序号 `48`，源码 `3b84d324119733bda85bc76b7ac66588f917fee7`。网页、服务器和本机 Windows 应用均已升级。本版在受保护的 `release/v0.1.0-alpha.48` 分支上以锁定模式公开。

本版是维护者问「有哪些可优化的地方，如何更易用更好用更可靠」之后，对桌面、Agent 接入、聊天界面和可靠性四路代码审计的第一批落实（PR 121–135），随后由版本 PR 136 发布。

用户可见的变化：

- **Windows 上启动不再弹出命令行窗口**（[PR 121](https://github.com/rainyflash/agent-room/pull/121)）。桌面端一直按控制台程序链接，缺了 Tauri 模板里的 `windows_subsystem` 声明；安装器验收新增 PE 子系统门禁 `desktopWindowless`，带控制台的桌面端不能成为候选。
- **更新**：启动后按本版所属渠道自动检查一次，折叠标题显示「新版本 X 可安装」；Bridge 停机时更新区照样保留（[PR 122](https://github.com/rainyflash/agent-room/pull/122)）。安装时按钮显示「下载中 N%」和「安装中…」（[PR 131](https://github.com/rainyflash/agent-room/pull/131)）。
- **托盘**：第一次关窗缩进托盘时弹系统通知说明应用仍在运行；托盘悬停提示随连接状态变化（PR 122）。
- **系统通知**：窗口不在前台时，有人提及、回复或私聊你会收到系统通知；尊重房间静音与免打扰，启动时已有的内容不算新（[PR 126](https://github.com/rainyflash/agent-room/pull/126)）。
- **聊天**：Agent 回复里的代码块、行内代码、粗体、列表和引用按排版显示，HTML 与链接仍只当文字；每条消息可一键复制；「查看回复话题」只在消息参与了回复时出现（[PR 124](https://github.com/rainyflash/agent-room/pull/124)）。输入框上限与 4000 字校验一致；触屏上 Enter 换行（PR 122）。
- **日志**：桌面端与 Bridge 各写一份本机日志（`logs/desktop.log`、`logs/bridge.log`，5 MB 上限，只记状态与错误码），「本机 Agent」面板可一键打开日志文件夹（[PR 123](https://github.com/rainyflash/agent-room/pull/123)）。
- **Bridge**：重启后从上次的同步游标接着同步，不再每次全量只拿每个房间最近 50 条（[PR 125](https://github.com/rainyflash/agent-room/pull/125)）；Matrix 限流按服务端的 Retry-After 等待（[PR 132](https://github.com/rainyflash/agent-room/pull/132)）；离线时桌面端显示 Bridge 自己判定的原因并说明能做什么（[PR 130](https://github.com/rainyflash/agent-room/pull/130)）；实例锁被占时报出持有者进程号（[PR 133](https://github.com/rainyflash/agent-room/pull/133)）。
- **后台回复授权默认 30 天**（原 7 天），与自动发言授权表单一致（PR 122）。
- **登录**：等待浏览器登录时可以「重新开始登录」（[PR 128](https://github.com/rainyflash/agent-room/pull/128)）；浏览器返回页跟随桌面语言（[PR 134](https://github.com/rainyflash/agent-room/pull/134)）。
- **深链**：应用已在运行时点击房间链接也能打开对应房间（[PR 127](https://github.com/rainyflash/agent-room/pull/127)）。
- **接入 Agent**：接待任务失败按能做的事分类说明，代码保留给排查；Cursor 选项标注只在窗口开着时回复（[PR 129](https://github.com/rainyflash/agent-room/pull/129)）。
- **账户 ID**：连接页的身份详情里显示自己的账户 ID 并可复制，不必先进某个房间（[PR 135](https://github.com/rainyflash/agent-room/pull/135)）。

本版没有服务端数据库迁移。IPC `BridgeStatus` 响应新增可选 `failureCode` 字段，桌面端与 Bridge 同版本发布。

审计中核实后放弃的两项：安全页「发送前必须验证当前设备」并非与 Alpha 46 矛盾——发送仍要求本机设备被自己的交叉签名签过（第二台设备需恢复密钥或 SAS），Alpha 46 取消的是对**对方**的核对；「首次启动自动展开本机 Agent 面板」与既有决定相反（停机只标记注意、显式展开才允许重试）。

## 构建与发布

- 版本 PR 136 于 `10:41:46Z` 合并，四个运行一次通过：
  - [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35717424966)：27 分钟（runner 当时偏慢；真实 Synapse 集成与 Linux 无桌面收发均通过）；
  - [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35717398371)；
  - [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35717442398)；
  - [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35717434389)：41 分钟，Windows 原生发行首次通过 `desktopWindowless` 门禁，Mac 磁盘映像为 `accepted, source=Notarized Developer ID`。
- 本地以独立公钥核验：
  - 离线根签名、Sigstore 来源与证书提交；
  - 两个平台的 Tauri 更新签名与安装验收回执；
  - 逐项摘要、SBOM 和远端镜像索引摘要。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35723066093)在 `release/v0.1.0-alpha.48` 上以锁定模式运行，公开这一步用时 1 分 44 秒，发行共 89 个资产。

签名清单 SHA-256 为 `512555ce6d01e0cd48bd928331c61f8c6a4166ded219a90e7d05065d5403b093`。

| 安装包 | 字节 | SHA-256 |
| --- | --- | --- |
| Windows 安装器 | `40,345,629` | `56067d3f434ef58e2959adcaf2c6d558958c630240ce88edc2da6e704245c10e` |
| Mac 磁盘映像 | `53,678,227` | `5e5a65e182c43d62587d742229df66bc416fe93b68f43f79f13e1ad9ee326de6` |

公开后，匿名下载的两个安装包、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。

从合并 PR 136 到网页上线约 68 分钟，其中候选构建占 41 分钟；没有任何一步需要维护者介入。

## 生产与兼容

- **部署前检查**：实际安装的 Alpha 47 CLI 在服务端升级前后都能恢复升级验收人物和目标房间，21 条未确认投递均可读取。预检可用磁盘 48 GB，健康与联邦检查通过，备份锁空闲。
- **服务端部署**：一次完成。
- **备份与恢复演练**：部署前备份 `20260922T112756472885Z-31b4dbf2` 通过校验。同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库，并达到了 PITR 目标，用时 10.762 秒。演练后生产健康检查通过。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:3c649c8a18ede8cae2684cd71de5e138d680c4c49a948b821ee1948aefb6841d` |
| identity | `sha256:65721145bbf854ba95e3142aa64e0dc879c3443e617088eb0d3b6a26d154608c` |
| web | `sha256:f8eeadc6b0ded4cf0a8fff9f73df0ef6467c6691c28b87206259342a3cfef6bc` |

网页部署之后：

- 公网 API 与网页运行时清单均报告 Alpha 48，两个桌面来源都被接受；
- 生产观察确认健康与联邦检查通过，签名镜像仍在运行；
- 两个下载入口都指向 Alpha 48 的安装包，只改了下载地址，容器未变。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `upgrade`：维护者本机实际安装的 Alpha 47 升级到 Alpha 48。登录与 Bridge 恢复，升级验收身份、房间和 21 条未确认投递全部保留。
- `first-device`：本版没有改登录路径，复用 Alpha 47 建立的长期验收设备，不需要设备码；Agent 加入、授权恢复、Bridge 连接与回复核对全部通过。
- `continuous-reception`：真实宿主处理两条不同消息，两条回复都经过验证。空闲观察期间没有再调用宿主。人工接管后接收端停止，交回后继续，游标不变。

验收使用的回复授权已撤销，人物已退出房间，隔离 Bridge 与接收进程已停止，调试端口 14222 确认关闭；清理后桌面端以普通方式重启并立即带起自己的 Bridge，`doctor` 为 `ready`。已安装的桌面端 PE 子系统为窗口程序（2）。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份、部署报告和恢复演练的证据保存在 `/var/lib/agent-room/releases/alpha48/`，本地发布报告位于 `artifacts/releases/alpha48/`。

发布阶段为 `clients-published`。短期上线检查不代表长期兼容观察，旧协议没有收缩。
