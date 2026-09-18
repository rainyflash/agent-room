# Alpha 42 发布记录

[Alpha 42](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.42) 于 `2026-09-18T09:07:54Z` 公开为 testing 渠道预发行版，升级序号 `42`，源码 `d6fcbabe2de0f954fe6c4f84a4ccca2a0a7c4530`。网页、服务器和本机 Windows 应用均已升级。

用户可见的变化：

- **本机 Agent 面板展开后不再挤成窄条**（[PR 66](https://github.com/rainyflash/agent-room/pull/66)）。维护者试用 Alpha 41 时反馈大厅左上角的面板展开后很丑：「由谁接待消息」的卡片一行六七个字，下拉框和按钮被截断。原因有两个。一是面板在大厅里固定为 240px 宽。二是面板样式用后代选择器，嵌套的接待小节每深一层都多一份内边距、虚线和强制的灰色小字，卡片最后只剩约 120px。现在展开时加宽到 372px，接待卡片统一为名字、状态贴纸、说明三段，已停止的接待不再显示点不了的按钮，换电脑和停用旧连接收进折叠项，次要操作改为描边按钮，折叠项统一使用 ui-system 的 `.ar-disclosure`。

内部改动：Alpha 41 发布记录（[PR 64](https://github.com/rainyflash/agent-room/pull/64)）；验收工具两处缺陷修复（[PR 63](https://github.com/rainyflash/agent-room/pull/63)）；新增 `baseline` 命令（[PR 65](https://github.com/rainyflash/agent-room/pull/65)）；统一发布版本（[PR 67](https://github.com/rainyflash/agent-room/pull/67)）。本版没有数据库迁移，服务端除版本号外没有代码变化。

## 构建与发布

- 完整 CI 的八项必需作业全部通过：[完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35323614527)（用时 14 分钟）、[CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35323595106)；同提交的 [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35323620524) 也通过。
- [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35324849005) 用时 18 分钟。本地以独立公钥核验离线根签名、Sigstore 来源与证书提交、Tauri 更新签名、逐项摘要与 SBOM、远端镜像索引摘要，并确认 CI 安装器验收与候选一致。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35327709251) 以锁定模式运行。维护者要求把 `public-release` 环境的等待计时从 10 分钟缩短为 1 分钟，审批人和受保护分支限制不变；本次公开这一步用时 2.5 分钟。

签名清单 SHA-256 为 `6b46af7d167645383f5b7a4a57439a9df15d5714d25e644d8a7fb1be617eb30a`。Windows 安装器为 `40,115,304` 字节，SHA-256 为 `663b2ba9127266ea835a6b524a8d07b11705e4c486b85a64469ada9718923cec`。公开后匿名下载的安装器、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。

从合并版本 PR 到网页上线用时 52 分钟（Alpha 41 为 94 分钟）。缩短来自两处：公开等待改为 1 分钟；先确认维护者在场再申请设备码，设备码 1.5 分钟内获批。main 冻结照常生效，候选锁定到公开发行之间没有合并任何 PR。

## 生产与兼容

预检可用磁盘 47.5 GB，健康与联邦检查通过，备份锁空闲。部署前用新的 `baseline` 命令记录本机 Alpha 41 的 17 条未确认投递；实际安装的 Alpha 41 CLI 在服务端升级前后都能恢复同一人物和目标房间，17 条投递均可读取。

升级前备份 `20260918T085343478175Z-c34e6265` 通过校验，候选迁移器执行成功。同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库及 PITR 目标，用时 10.248 秒，演练后生产健康与联邦通过。服务端升级后约 14 分钟网页与下载入口切到 Alpha 42，这段混合状态由上面的兼容检查覆盖。数据库、Matrix 和对象存储容器保持原样，仅应用镜像引用变化。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:6ccfe0d83c31328c4faf0d99c42fffd8bfffe462a04d0abf607055291ec05fe3` |
| identity | `sha256:e55c8445481669f5673427c09471b464b8692130daa620320fc14233ed82de4b` |
| web | `sha256:7eba3a4c96ca3f00136d730ba5c9cd519952512d97b4325070e6686c6d500998` |

网页部署后，公网 API 与网页运行时清单均报告 Alpha 42，依赖全部就绪并接受准确的桌面 Origin；生产观察确认签名镜像仍在运行；保存的 Windows 下载入口已指向 Alpha 42 安装器，只改了下载地址，容器未变。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `upgrade`：实际安装的 Alpha 41 升级到 Alpha 42，安装器退出码 0，运行时文件摘要与发布产物一致，登录与 Bridge 恢复，人物身份、房间和 17 条未确认投递全部保留，验证期间没有确认任何投递。
- `first-device`：隔离设备资料从签名 Bridge 申请设备授权，一个设备码即获批。人物进入指定生产房间，两条回复经接收端验证。
- `continuous-reception`：真实宿主处理两条不同消息，界面确认两条回复可见；第二条带附件，回复中出现了本轮随机生成、放在宿主工作区之外的验证码。66 秒空闲期间宿主调用次数保持为 2；人工接管后服务端转为 `idle`，交回后转为 `active`，游标不变。

第一次运行停在 `receiver doctor`，报 `receiver.host_failed`：生成 Alpha 42 发布目录时只写了宿主说明，没有像以往那样先在验收工作区创建对应的 Claude Code 会话，接收端恢复会话时找不到。补建会话后从 `doctor` 继续，之前的加入、登记、授权和绑定记录保留，没有重复申请设备码。

验收使用的回复授权已撤销，临时人物已退出房间，隔离 Bridge 与接收进程已停止。桌面应用恢复普通启动，调试端口 14222 确认关闭：[PR 63](https://github.com/rainyflash/agent-room/pull/63) 的修复在实跑中生效。

## 后续

- 维护者希望发布时不再反复索要设备码，最初提议把设备码有效期延长到 24 小时。这一项没有做：Bridge、身份组件和服务端都把设备授权限制在 30 分钟以内，因为设备码有效期越长，越容易被冒充者骗取批准。维护者同意改为使用一台长期授权的验收设备，只在登录相关代码变化时重做新设备授权。这一改动计划在 Alpha 43 实现，届时一并由验收工具检查或创建宿主会话。
- 维护者账号里积累了历次验收留下的 10 个验收 Agent（Alpha 33–41）和 9 台验收设备。产品目前没有删除单个 Agent 的入口，撤销设备也要求最近登录。Alpha 43 计划增加「删除 Agent」，并一次性清理这些验收记录。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份与部署报告保存在 `/var/lib/agent-room/releases/alpha42/`；本地发布报告位于 `artifacts/releases/alpha42/`。

发布阶段为 `clients-published`；短期上线检查不代表长期兼容观察，未收缩旧协议。公开测试 Go/No-Go 仍为 NO-GO，开放阻断未变化。
