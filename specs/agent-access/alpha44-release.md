# Alpha 44 发布记录

[Alpha 44](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.44) 于 `2026-09-19T07:20:36Z` 公开为 testing 渠道预发行版，升级序号 `44`，源码 `e51fa41e67008d1fa0a0626c0f6bc11db01e3054`。网页、服务器和本机 Windows 应用均已升级。这是第一个在受保护的 `release/v0.1.0-alpha.44` 分支上以锁定模式公开的版本，发布期间 main 没有冻结。

用户可见的变化：

- **登录、注册、设备批准与邮件改为游戏大厅风格**（[PR 75](https://github.com/rainyflash/agent-room/pull/75)）。这些页面由身份服务渲染，此前一直是 Keycloak 自带的深色英文界面。现在与网页和桌面应用同一套风格，默认简体中文，可切换英文。
  - 注册只需邮箱和昵称，验证邮箱后再设置密码。
  - 设备批准页会显示设备码，请用户与电脑上显示的码核对：Bridge 把设备码放进验证链接的 URL 片段，经过登录跳转也不会丢。
  - 验证邮件和重设密码邮件使用同一版式。
- **「接入 Agent」弹窗在 Agent 进入房间后不再显示过时的粘贴提示**（[PR 78](https://github.com/rainyflash/agent-room/pull/78)）。这是在新服务器上建立升级验收人物时发现的。
- **本地消息存储的锁竞争不再误报为不可用**（[PR 74](https://github.com/rainyflash/agent-room/pull/74)）。SQLite 写事务在开头就取写锁。Alpha 43 的完整 CI 曾因此偶发失败。

内部改动：Alpha 43 发布记录（[PR 76](https://github.com/rainyflash/agent-room/pull/76)）；公开发行改在钉住候选的受保护 release 分支上运行（[PR 77](https://github.com/rainyflash/agent-room/pull/77)）；统一发布版本（[PR 79](https://github.com/rainyflash/agent-room/pull/79)）。身份主题和身份镜像的改动现在也要求重做新设备授权。本版没有服务端数据库迁移。

## 构建与发布

- [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35425669760) 一次通过八项必需作业，用时 14 分钟；[CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35425656486) 和 [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35425656481) 通过。版本 PR 合并前，在 main 上提前跑的一次全量 CI 里，真实网页登录作业已在新登录页上通过。
- [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35425673764) 两次尝试：
  - 第一次，Windows 原生发行作业在安装 Cosign 时失败，runner 报 `rm: cannot remove 'cosign.exe': Device or resource busy`，与代码无关。重跑后通过。
  - 第二次，签名候选已完整归档，但创建草稿发行时返回 `HTTP 403: Resource not accessible by integration`。原因是版本提交之后我合入了 [PR 80](https://github.com/rainyflash/agent-room/pull/80)，它改了 `ci.yml`：在工作流文件与默认分支不同的提交上创建发行或标签，GitHub 要求令牌有 `workflow` 权限，`GITHUB_TOKEN` 和维护者当前的 gh 令牌都没有。处理办法：
    1. 用 [PR 81](https://github.com/rainyflash/agent-room/pull/81) 临时撤回 PR 80，让 main 的工作流文件与候选提交一致；
    2. 按发布流程的恢复路径，从归档恢复已签名的完整候选，独立核验后创建草稿，并上传 50 个资产；
    3. 逐个核对草稿资产摘要与核验过的文件一致。整个过程没有重新签署。
- 本地以独立公钥核验离线根签名、Sigstore 来源与证书提交、Tauri 更新签名、逐项摘要与 SBOM、远端镜像索引摘要，并确认 CI 安装器验收与候选一致。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35428961469) 在 `release/v0.1.0-alpha.44` 上以锁定模式运行，运行提交与候选提交一致，标签由工作流创建在候选提交上。公开这一步用时 2 分 19 秒。

签名清单 SHA-256 为 `2f4b835c5f694c3723bfaeb1dc03408f5f573908077705f89e39e425608237d7`。Windows 安装器为 `40,124,560` 字节，SHA-256 为 `ebebf385184d04e6b1cf8b66c1daef6d7cc9e4b2198b50d80f3bc0726c4b18b3`。公开后，匿名下载的安装器、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。

从合并版本 PR 到网页上线用时约 1 小时 20 分钟。大部分时间花在两处候选恢复、下面的部署中断，以及等维护者到场批准设备码上。

## 生产与兼容

- **部署前检查**：用新建的升级验收身份记录了 5 条未确认投递（见下文）。实际安装的 Alpha 43 CLI 在服务端升级前后都能恢复同一人物和目标房间，5 条投递均可读取。预检可用磁盘 42 GB，健康与联邦检查通过，备份锁空闲。
- **部署中途停下**：部署在备份之后停下，报「部署期间配置或密钥变化」。原因是生产源码切到本版后，每 15 分钟一次的定时备份用新渲染器重新渲染了部署配置：PR 75 给 Realm 生成文件加了主题、语言和密码策略，Realm 文件与 `compose.env` 里的生成配置摘要随之改变。
- **核对后续跑**：用脚本逐项核对，Realm 文件相对部署自己的备份只多出 PR 75 的 6 个键，其余受摘要覆盖的文件（部署配置与密钥）均未改变；随后把检查点的配置基线移到新渲染结果，证据保存在服务器的 `releases/alpha44/configuration-rerender.json`，续跑后部署完成。[PR 82](https://github.com/rainyflash/agent-room/pull/82) 让以后的部署在备份之后先用候选代码渲染一次，并在公开前拒绝工作流文件已变化的情况。
- **备份与恢复演练**：部署前备份 `20260919T064441937163Z-4e8cb15c` 通过校验。同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库及 PITR 目标，用时 8.152 秒，演练后生产健康通过。
- **Realm 设置**：服务端部署在身份服务升级后运行一次 Realm 协调，把已有 Realm 切到新主题与中英双语。生产登录页确认已是新样式、默认简体中文。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:8ce86a41adac0311920408c72bdbf84c6e91628bf36cd3df30031a159611649b` |
| identity | `sha256:5907366caa5ba338cd27c795beb13f3400bb341f83382c5906917dbd6b3ebb21` |
| web | `sha256:623ba8ecfbe63a8b2623d19cd7630bda4eebea670222bd0f28754bc01cffb28c` |

网页部署后，公网 API 与网页运行时清单均报告 Alpha 44，依赖全部就绪，并接受准确的桌面 Origin。生产观察确认健康与联邦检查通过，签名镜像仍在运行。保存的 Windows 下载入口已指向 Alpha 44 安装器，只改了下载地址，容器未变。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `upgrade`：维护者本机实际安装的 Alpha 43 升级到 Alpha 44，安装器退出码 0，运行时文件摘要与发布产物一致。登录与 Bridge 恢复；升级验收身份、房间和 5 条未确认投递全部保留，验证期间没有确认任何投递。
  - 原来的升级验收身份在已归档的旧服务器上。本版之前，用已安装的 Alpha 43 CLI 在新服务器的验收房间里新建了人物「升级验收」，并留下一条它未读的房间消息作为未确认投递。
- `first-device`：本版改了身份主题，属于登录相关代码，因此按规则重做新设备授权。隔离设备资料从签名 Bridge 申请设备授权；维护者在新的设备批准页上核对并批准了一个设备码，一次成功。人物进入新服务器上的验收房间，两条回复经接收端验证。这台设备现在是长期验收设备。
- `continuous-reception`：真实宿主处理两条不同消息，两条回复经验证；第二条带附件，回复里出现了本轮随机生成、放在宿主工作区之外的验证码。66 秒空闲期间，宿主调用次数保持为 2。人工接管后服务端转为 `idle`，交回后转为 `active`，游标不变。

验收使用的回复授权已撤销，人物已退出房间，隔离 Bridge 与接收进程已停止。桌面应用恢复普通启动，调试端口 14222 确认关闭。

## 迁移收尾

- 维护者同意后，删除了 Cloudflare 上 `the-zeroth.com` 区域里 `room.the-zeroth.com` 的 8 条记录：5 条 A 记录和旧 Resend 发件域名的 3 条记录。删除前的完整内容保存在 `artifacts/releases/alpha43/cloudflare-room-records-deleted.json`。区域里其他 18 条记录未动，公共 DNS 确认这些名字已返回 NXDOMAIN。
- 旧 Resend 域名在另一个账号里，需要维护者自行移除。归档的旧部署与备份仍然保留。

## 后续

- **恢复 PR 80**：标签已创建，PR 80 通过 [PR 84](https://github.com/rainyflash/agent-room/pull/84) 恢复。发布期间不要合并改动 `.github/workflows` 的 PR；发布流程文档和调度器已写明并检查这一点。
- **部署渲染修复的生效时间**：PR 82 的渲染顺序修复从下一个版本起生效，因为部署工具从发布提交本身运行。
- **身份服务不可达时不再退出**：Alpha 43 候选的安装器验收曾因新域名尚未上线而失败。[PR 83](https://github.com/rainyflash/agent-room/pull/83) 让首次设备授权连不上身份服务时进入可恢复的等待，而不是退出；它在版本提交之后合入，随下一个版本发布。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份、部署报告和配置重新渲染的证据保存在 `/var/lib/agent-room/releases/alpha44/`，本地发布报告位于 `artifacts/releases/alpha44/`。

发布阶段为 `clients-published`。短期上线检查不代表长期兼容观察，旧协议没有收缩。
