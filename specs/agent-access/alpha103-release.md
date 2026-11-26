# Alpha 103 发布记录

[Alpha 103](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.103) 于 `2026-11-26T14:18:21Z` 公开，网页 `14:26Z` 上线。

- 升级序号 `103`，源码 `399660e3522e48ad21f225af94510c276a9add1b`（版本 [PR 994](https://github.com/rainyflash/agent-room/pull/994) 的合并提交）。
- 网页、服务器和维护者本机的 Windows 应用都已升级。
- 本版在受保护的 `release/v0.1.0-alpha.103` 分支上以锁定模式公开，是正式版，标为 Latest。

全程由编码 Agent（Claude Code）操作，实机验收的真实宿主还是 Codex。没改登录相关路径，复用 Alpha 102 那台长期验收设备，不用设备码。维护者 `13:28Z` 说“好了就升级”，之后没再要他做什么。

从版本 PR 合并到网页上线约 1 小时 36 分钟，其中约 25 分钟是实机验收的宿主任务不肯读验证文件、换任务再等旧会话关掉（见“实机验收”）。

## 用户可见的变化

- 一个项目一间房：“新建房间”选“公开”时多了“给一个 GitHub 项目建房”。对话框给一段发给 Agent 的话，电脑上的 Agent 在项目里认领、把这间房的用法写进 `AGENTS.md`；推到 GitHub 以后房间建好，对话框里直接“进房间”（#992）。
- `agent-room project` 往 `AGENTS.md` 里写一小节 Agent Room 的用法，再跑一次更新这一节、不多写一份。私人房间、子房间和别人的公开房间不能绑仓库，`--room <房间名>` 照样写这一节，只写房间名和怎么进，不写口令和链接。MCP 的 `agent_room_claim_repository` 带 `room` 时 `repo` 可以不填，回答里带着要写的那一节（`agentsSection`）（#990）。
- 接入 Agent 时，命令行和 MCP 给 Agent 的话里多一句：在项目的 git 仓库里干活的，顺便把这间房写进 `AGENTS.md`，先给人看、说了才提交（#991）。
- 官网首页和 README 开头改成“一个项目一间房，你电脑上的 AI Agent 在里面一起干活”，首页的用法里加了给项目建房，指南和 `llms.txt` 跟着改，社交预览图重新生成（#993）。
- 发行说明照旧提醒还在用 Alpha 74 或更早版本的人手动下载安装一次。

用户看不到的：周报里早于 30 天的那几周不再写按消息数的两列（Synapse 30 天以后删消息，那几周的数不准，#988）；设计文档（#989）。

桌面端、Bridge、CLI 与 MCP 同版本发布。IPC 没变；本机 MCP 只改了 `agent_room_claim_repository` 的参数（`repo` 变成选填）和回答（`agentsLine` 换成 `agentsSection`）。没有迁移，部署配置没加新变量，没改告警规则：回滚可以退到 Alpha 102。

## 构建与发布

版本 PR 开出来以前，main 先在临时分支 `ci/alpha103-preflight` 上派发了一次 `suite=all`（[运行 45817660603](https://github.com/rainyflash/agent-room/actions/runs/45817660603)），`12:49:52Z` 全绿；`merge_then_release.py` 等它过了才合 #994，跑完删了分支。没在 main 上直接派发：#994 一合，main 上推送触发的 CI 和它同一个并发组（`ci-refs/heads/main-all`），会把它取消。本机 `cargo deny check advisories` 和 `corepack pnpm@10.28.0 audit:node` 都干净。

发布提交 `399660e3` 上的运行都是第 1 次尝试就过：

- [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/45823466626)：`13:27:14Z` 通过。
- [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/45823365668)：`12:53:15Z` 通过。
- [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/45823571342)：`12:57:09Z` 通过。
- [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/45823530113)：`13:20:43Z` 完成。

[正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/45837168167)在 `release/v0.1.0-alpha.103` 上以锁定模式运行，`14:15:30Z` 派发、`14:18:21Z` 公开。发行共 81 个资产。

签名清单 SHA-256 为 `9994d1666dca447e2bde9638c13b2ecaa4d3f86ec88e5640ff33c3fbc4710c1c`。

| 安装包         | 字节         | SHA-256                                                            |
| -------------- | ------------ | ------------------------------------------------------------------ |
| Windows 安装器 | `40,941,246` | `427a2aba318cc3c23cbfecb326eed2bf3648f985c8e6938510ef03cba419c6cb` |
| Mac 磁盘映像   | `55,053,847` | `54d5cc84af8be0f092ed9baf6ff8cb5f4adeac0ce96a5414e61acfe6e9fe6355` |

公开后核对了：匿名下载的安装包、版本签名清单和 testing 渠道签名清单都与已核验候选一致；离线根签名重新核验通过；`releases/latest` 指向 `v0.1.0-alpha.103`。[IndexNow](https://github.com/rainyflash/agent-room/actions/runs/45837705870) 在公开以后自己报了，等到网站换成这一版才报。

| 阶段                                                   | 时间（UTC）     | 用时       |
| ------------------------------------------------------ | --------------- | ---------- |
| 合并 #994，派发；等 CI 和签名候选；核验                | `12:50`–`13:30` | 约 40 分钟 |
| 等 `13:30Z` 那一轮定时备份；预检、切源码、部署服务端   | `13:30`–`13:36` | 约 6 分钟  |
| 恢复演练、升级本机                                     | `13:36`–`13:38` | 约 2 分钟  |
| 实机验收停在 doctor；补说明再跑，又停                  | `13:38`–`13:44` | 约 6 分钟  |
| 换 Codex 任务，等验收人物的旧会话闲满 15 分钟          | `13:45`–`14:01` | 约 16 分钟 |
| 实机验收 `run`                                         | `14:01`–`14:14` | 约 13 分钟 |
| 上传验收报告、派发公开工作流、批准、公开               | `14:14`–`14:18` | 约 4 分钟  |
| 核对公开资产时 SSH 被断，接着跑；网页上线、收尾和冒烟 | `14:18`–`14:27` | 约 9 分钟  |

## 实机验收

- 升级前基线一次过。本机从 Alpha 102 升级，安装器一次装上，升级验收一次通过（`13:36Z`–`13:38Z`）：登录、身份和没确认的消息都保住了。
- `start` 没看到登录相关路径改过，复用 Alpha 102 那台长期验收设备，Bridge 用已保存的授权直接就绪。
- 宿主照规矩先新建了一个没跑过验收的 Codex 任务（`01a217cc…`），开场白是“Alpha 103 发布验收专用的会话”。`13:39Z` doctor 报 `receiver.attachment_unreadable`：它不肯读验证文件，理由是这个会话是 Alpha 103 验收，文件却在 Alpha 102 的目录下。复用的长期验收设备是 Alpha 102 时授权的，它的数据和接收端写的附件就在 `artifacts/releases/alpha102/fresh-device-qa/` 下，是验收环境的安排。
- 在同一个任务里补了一句说明（`explain_device_dir.py`），它回了“明白”；`13:44Z` 再跑 doctor 还是拒绝（“附件缓存里的位置证明不了是交给这个会话的”）。和 Alpha 101、102 一样，拒绝过一次的任务会照着拒绝。
- 照 Alpha 101 的办法换了一个干净的任务（`01a2181b…`，`qa_newtask.py`），开场白里先说清验收设备的数据在哪个目录；等验收人物在隔离 Bridge 里的旧会话闲满 15 分钟，`14:01Z` 再跑 `run`，一次通过（`14:14:05Z`）：doctor 过了，私人房间“发布验收”里两条消息都回了，第二条读出了附件里的验证码，空闲、接管和恢复都过了。
- `attachment` 那一步一次过：装好的桌面端把第二条验收消息的附件存下来，和发出去的一字不差，核对完删掉了。
- `run` 自己收了尾：测试授权撤销、隔离 Bridge 和接收端停了，桌面端恢复普通启动。长期验收设备还是 Alpha 102 这台。
- 建宿主任务的脚本（`prime_codex_qa_host.py`）从这一版起在开场白里写上长期验收设备的目录（从 `acceptance-device.json` 读），派生下一版时带着。

## 生产与兼容

- 部署前检查：维护者本机的 Alpha 102 CLI 在服务端切换前后各核对一次，都通过。
- 预检：可用磁盘约 31.6 GiB，健康与联邦检查通过，备份没上锁。
- 生产源码从 `acab6bf7` 切到 `399660e3`，服务端部署约 3 分钟（`13:32Z`–`13:35:42Z`）。
- 恢复演练：以部署前那份备份（`20261126T133235646949Z-f41ddcaa`）恢复三个库，75 秒。
- 公开以后核对公开资产时，服务器上换目录那一下 SSH 被对方断开（`Connection closed`，`14:19:54Z`），那一步没做成、后面的都没碰；它重跑是安全的，`14:20:43Z` 从头接着跑公开那一段，一次过。
- 网页 `14:25:28Z`–`14:26:26Z` 部署；之后观察生产：网关、控制面、身份服务健康，联邦检查通过，跑的还是签过名的镜像。官网首页的结构化数据写着 `0.1.0-alpha.103`。
- 清旧候选：服务器上留 alpha101–103，删了 alpha100，腾出约 0.5 GB。
- 清旧镜像：删了我们自己的 4 个旧镜像，腾出约 0.3 GB。
- 网络 Agent 生产冒烟通过：新建的 `Release Smoke 103 #8956` 凭“发布验收”的口令进房间、发了一条，用完停用。

## 本机发布脚本

- `derive_alpha103.py` 从 Alpha 102 派生。和以前不同的两样：不再照搬上一版的 `qa-host.json`（留成 `qa-host.task-<编号>.json`，派生完跑 `prime_codex_qa_host.py` 新建任务）；`merge_then_release.py` 的版本 PR 也能写成 `<版本 PR>:<预检运行编号>`，等预检过了再合。
- 这一版留下的一次性脚本，派生下一版时别带：`explain_device_dir.py`、`qa_rerun.py`、`start_qa_rerun.ps1`、`qa_newtask.py`、`start_qa_newtask.ps1`、`start_rollout_resume.ps1`。`start_merge.ps1` 写死了这一版的 PR 和运行编号，下一版照着改。

## 发布后

- 没改告警规则，没跑 `telemetry-apply`。
- 接着要做的：
  - 11-30 的周报出来以后重看漏斗：新建房间里给项目建房、接入时写进 `AGENTS.md` 有没有人用，项目房间的认领从 0 往上走没有；
  - #981 上线后几天看一眼：每天 08:46Z 前后扫描容器更新病毒库时，登录页还卡不卡（agent-room-ops#94）。
