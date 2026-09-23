# Alpha 49 发布记录

[Alpha 49](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.49) 于 `2026-09-23T06:12:40Z` 公开为 testing 渠道预发行版，升级序号 `49`，源码 `b3595291a4274bf68f89075d0d7796022991300b`。网页、服务器和本机 Windows 应用均已升级。本版在受保护的 `release/v0.1.0-alpha.49` 分支上以锁定模式公开。

本版的主题是**接入 Agent 不用再复制**：维护者问“接入 Agent 能不能不用一段段复制，配了 MCP、公开房间和有权限的房间也得复制吗”之后的前两层方案（PR 138–141），加上 Alpha 48 体验清单里剩下的几项（PR 142–145）。版本 PR 146 之后的第一个候选在实机验收中作废（见下文「作废的候选」），PR 149 修好原因后重建；其间合入的 PR 147 一并发布。

用户可见的变化：

- **一键给 Claude Code 装技能**（[PR 138](https://github.com/rainyflash/agent-room/pull/138)）：接入面板里点一次，把 agent-room 技能装进 `~/.claude/skills/agent-room/`（或 `CLAUDE_CONFIG_DIR`），接入说明从约 2,200 字缩成几行；Claude Code 不用重启就能读到。装进去的技能末尾写明这台电脑的 CLI 前缀（程序位置、数据目录、连接命名空间），桌面端启动时自动把已装的过期技能更新到本版（[PR 140](https://github.com/rainyflash/agent-room/pull/140)）。
- **按房间名接入**（[PR 139](https://github.com/rainyflash/agent-room/pull/139)、[PR 140](https://github.com/rainyflash/agent-room/pull/140)）：控制面新增设备签名的 `GET /lobbies/accessible`，列出账号能进的公开大厅与受邀或已加入的私人房间；Bridge 通过 IPC `ListRooms` 提供给 CLI 与 MCP。CLI `agent-room rooms` 与 `agent-room join --room "<名字或 slug>"`，MCP `agent_room_list_rooms` 与 `agent_room_join`。找不到或同名多间时失败并列出候选，不会改进别的房间；人物按宿主任务保存（Codex 的 `CODEX_THREAD_ID`、Claude Code 的 `CLAUDE_CODE_SESSION_ID`），同一任务再次接入同一房间找回原人物。
- **接入面板“等待接入”**（[PR 141](https://github.com/rainyflash/agent-room/pull/141)）：面板开着时把面板里的人物挂在本机 Bridge 上（只在内存里，十分钟不续期就失效）；已装技能或配好 MCP 的 Agent 听到“接入 Agent Room”就接上这份邀请，面板上立刻显示它进来了。不带参数的 `join` / `agent_room_join` 依次接面板的邀请、回到这个任务上次的人物、进默认公开大厅。
- **补回离线期间漏掉的消息**（[PR 142](https://github.com/rainyflash/agent-room/pull/142)）：同步时一个房间一次来得太多（超过 50 条）时，Bridge 从 Matrix 给的令牌往回翻页补齐，碰到已经记下的事件就停，最多五页；补回的消息按时间先后走同样的验签，收件箱把它们当作新到的消息交给 Agent。
- **暗色模式**（[PR 143](https://github.com/rainyflash/agent-room/pull/143)）：跟随系统设置；只换设计令牌，彩色底一起压暗，游戏场景保持白天配色；系统是暗色时桌面窗口底色也是暗色，加载前不闪白。
- **私人房间改名**（[PR 144](https://github.com/rainyflash/agent-room/pull/144)）：房主在房间设置里改名；先改 Matrix 房间名再写目录，所有客户端和“按名字接入”都用新名字。
- **网络一恢复就重连**（[PR 145](https://github.com/rainyflash/agent-room/pull/145)）：Bridge 退避重连时先探一下 Matrix 服务器，连不上就每 3 秒再探，网络一恢复立刻重连，不再干等最长 60 秒的退避。
- **Windows 具名管道加固**（PR 145）：本地客户端把每个连接整个放到一个专用线程上（mio#2011），MCP 服务端与桌面壳不再暴露于“丢弃连接与 I/O 驱动并发踩堆”的无声崩溃。
- **macOS 上不再反复要钥匙串密码**（[PR 147](https://github.com/rainyflash/agent-room/pull/147)）：Bridge 的两项 IPC 凭据此前只信任 Bridge 自己，桌面、MCP 和命令行每读一次就弹一次登录钥匙串密码。现在 Bridge 创建这两项时就信任同一安装里的这三个程序，每次启动按原值重建一次，已装旧版的凭据也随之迁移；Unix 上客户端先连上 Bridge 再读凭据，读到的总是重建过的那份。只对团队签名的正式构建有效，Windows 不变。
- **自动回复不再因代码块作废**（[PR 149](https://github.com/rainyflash/agent-room/pull/149)）：Claude Code 宿主有时把 `{"body": …}` 整段包进 ```` ```json ```` 代码块，接待端按无效回复处理，整轮自动回复作废。现在恰好一整块、信息串为空或 `json` 的代码块会先拆开再按原规则校验；代码块前后有其他文字、多块或多余字段仍按无效处理。契约检查的提示词也写明不要代码块。这是本版实机验收发现的。

本版没有服务端数据库迁移。控制面新增 `GET /lobbies/accessible`（设备签名）与 `PUT /private-rooms/{catalog_id}/name`；IPC 新增 `ListRooms`、`OfferInvitation`、`WithdrawInvitation`、`ReadInvitation`，`IpcHostRoomTarget.room_id` 改为可选；MCP 新增两个工具（共 16 个）。桌面端、Bridge、CLI 与 MCP 同版本发布。

## 构建与发布

- 修复 PR 149 于 `05:13:05Z` 合并，随即在 main 的 `b359529` 上派发：
  - [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35821540302)：23 分钟，真实 Synapse 集成与 Linux 无桌面收发均通过；
  - [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35821503588)；
  - [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35823276174)：PR 149 没改联邦相关路径，推送时不会自动运行，手动派发后通过；
  - [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35821546611)：原生产物与三套镜像 37 分钟内全部通过，Mac 版照常签名与公证。汇总作业把完整签名候选保存为 Actions 归档之后，上传草稿时失败：作废候选的草稿还占着 `v0.1.0-alpha.49` 这个标签。重跑汇总作业也不行，它会拿到第一次保存的归档并拒绝覆盖元数据。
- 按 `docs/operations/signed-releases.md` 的恢复流程处理：下载那份归档，以同样的密码学门禁核验后恢复上传，不重新编译或签署。本地以独立公钥核验：
  - 离线根签名、Sigstore 来源与证书提交；
  - 两个平台的 Tauri 更新签名与安装验收回执；
  - 逐项摘要、SBOM 和远端镜像索引摘要。

  核验通过后新建草稿（目标 `b359529`），77 个资产逐个按 SHA-256 与本地候选核对（`artifacts/releases/alpha49/resume_candidate_upload.py`）。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35825494186)在 `release/v0.1.0-alpha.49` 上以锁定模式运行，发布前核对草稿签名清单与已核验候选逐字节一致，发行共 89 个资产。

签名清单 SHA-256 为 `8ac2c9f0303a37e8337a2bea3a74f4ec5941e94036b072ec023b35a0d9be7a17`。

| 安装包 | 字节 | SHA-256 |
| --- | --- | --- |
| Windows 安装器 | `40,690,524` | `08c00b4824de8d602fddc7366f88e83a40524fe6d02ed9855bb71032ad70d5dd` |
| Mac 磁盘映像 | `53,949,120` | `99506ec0754dbeb126125f525f33e3a268ba7aad21bba05814861ab1ec47b1e9` |

公开后，匿名下载的两个安装包、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。

从合并 PR 149 到网页上线约 62 分钟，其中候选构建占 37 分钟；没有任何一步需要维护者介入。

## 生产与兼容

- **部署前检查**：维护者本机装回的 Alpha 48 CLI 在服务端切换前后都能恢复升级验收人物和目标房间，25 条未确认投递均可读取。服务端切换前生产运行的是作废候选（见下文），公网 API 前后都报告 `0.1.0-alpha.49`，两次构建靠源码切换与部署报告区分。
- **预检**：可用磁盘 45 GB，健康与联邦检查通过。06:00 的定时备份结束、备份锁空闲后才开始切换，避开了作废候选部署时撞上的定时备份。
- **服务端部署**：生产源码从 `d6e9fe5` 切到 `b359529`，部署一次完成，约 1.5 分钟。
- **备份与恢复演练**：部署前备份 `20260923T060137442365Z-07a6479b` 通过校验。同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库，并达到了 PITR 目标，用时 11.694 秒。演练后生产健康检查通过。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:7552098417a6c6154d1e15138642e3606ebd8bf2c011a37bddf94d323c8a13a3` |
| identity | `sha256:71402658d90be7ade2f365692e07d2a98f170f57b120130b99f7788ee1c7dce4` |
| web | `sha256:4af7623344ad39a8c140a494278ee48bc0d48d58a1fa4b8d23d0716505d59490` |

网页部署之后：

- 公网 API 与网页运行时清单均报告 Alpha 49，两个桌面来源都被接受；
- 生产观察确认健康与联邦检查通过，签名镜像仍在运行；
- 两个下载入口都指向 Alpha 49 的安装包，只改了下载地址，容器未变。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `upgrade`：维护者本机先用 Alpha 48 已核验的安装器装回 Alpha 48（见下文），再原地升级到本候选。登录与 Bridge 恢复，升级验收身份、房间和 20 条未确认投递全部保留。
- `first-device`：本版没有改登录路径，复用 Alpha 47 建立的长期验收设备，不需要设备码；Agent 加入、授权恢复、Bridge 连接与回复核对全部通过。
- `continuous-reception`：契约检查这次宿主直接回了 JSON。真实宿主处理两条不同消息，第二条带附件，宿主读出了其中的验证码；两条回复都经过验证。空闲观察期间没有再调用宿主。人工接管后接收端停止，交回后继续，游标不变。

验收使用的回复授权已撤销，人物已退出房间，隔离 Bridge 与接收进程已停止，调试端口 14222 确认关闭；清理后桌面端以普通方式重启并带起自己的 Bridge，`doctor` 为 `ready`。

## 作废的候选

版本 PR 146 合并后的候选 **`d6e9fe5`** 已通过完整 CI 与签名核验，服务器已部署到这一版并通过旧客户端检查与恢复演练，维护者本机也已从 Alpha 48 原地升级，`start` 复用了长期验收设备。`run` 的契约检查两次失败，报 `receiver.host_reply_invalid`：Opus 宿主把 `{"body": …}` 包进了 ```` ```json ```` 代码块，重试时宿主照着会话里上一次的回复又包了一次。同样的回复在真实自动回复里会让整轮作废，所以先修再发（PR 149），不带着这个问题公开。

- 这次尝试建的自动回复授权已撤销，长期验收设备的人物已退出房间，候选的隔离 Bridge 已停止，桌面应用恢复普通启动。
- 本地记录移到 `artifacts/releases/alpha49/attempt-d6e9fe53/`，服务器上的记录改名为 `releases/alpha49-attempt-d6e9fe53`，都保留。
- 候选工作流会创建私有草稿。作废候选的草稿改标签为 `v0.1.0-alpha.49-superseded-d6e9fe5`；重建时第一次上传失败留下的空草稿改标签为 `v0.1.0-alpha.49-empty-draft-b359529`。两份都保留，待维护者决定是否删除。
- 维护者本机已装着这个候选，而升级验收要求先装着上一个公开版本，所以重建后用 Alpha 48 已核验的安装器装回 Alpha 48，再重新记录升级前基线。Alpha 48 与 49 之间没有本地数据库迁移，人物档案格式也没变。
- `04:44Z` 到 `06:01Z` 之间生产服务器运行的是这个作废候选的源码，随后由本版切换替换。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份、部署报告和恢复演练的证据保存在 `/var/lib/agent-room/releases/alpha49/`，本地发布报告位于 `artifacts/releases/alpha49/`。

发布阶段为 `clients-published`。短期上线检查不代表长期兼容观察，旧协议没有收缩。
