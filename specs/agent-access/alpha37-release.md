# Alpha 37 发布记录

[Alpha 37](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.37) 于 `2026-09-16T09:29:22Z` 公开为 testing 渠道预发行版，升级序号 `37`，源码 `4908f724d5da26f10d9c58cd5e7460d2b6f653f3`。网页、服务器和本机 Windows 应用均已升级。

本次修复后台接待的宿主读不到附件的问题。已校验附件此前下载到系统临时目录，而 Claude Code 以 `--restricted` 启动只能读取工作目录，读取被拒绝会记为权限拒绝并使整轮接待失败，回复不会发出。附件改为下载到 Bridge 数据目录下的私有 `attachments` 目录，启动宿主时按 `--add-dir` 只对该目录授予只读访问；目录名由 `agent-room-bridge-ipc` 统一给出，Bridge 与接待端不会漂移。宿主能力检查同时要求 `--add-dir`，旧版本继续返回 `receiver.claude_upgrade_required`。进程异常退出遗留的附件在准备运行目录时清理。

Codex 的只读沙箱可以读取工作目录之外的路径，因此该缺陷只在 Claude Code 宿主上出现；此前各版本均以 Codex 完成接待验收，没有暴露它。

Alpha 33 至 Alpha 36 的候选均未公开，本次统一以 Alpha 37 发布，上一公开版本仍为 Alpha 32。

## 构建与发布

- [发布准备 PR 37](https://github.com/rainyflash/agent-room/pull/37) 正常合并到受保护主分支。
- [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35072874300) 的八项完整验证通过。首次运行时 `PostgreSQL、Matrix、对象存储与协议集成` 失败于既有的偶发测试：`second_device` 夹具在同一条 INSERT 里调用三次 `clock_timestamp()`，时钟在调用之间前进时 `last_seen_at` 会早于 `created_at`，违反 `device_timestamp_order` 约束。该失败与本次改动无关，重跑该作业后通过，夹具修复另行处理。
- 同提交的 [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35072847116) 与 [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35072850327) 通过。
- [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35072882649) 包含 Windows 桌面、Bridge、CLI、MCP、插件及三套 amd64/arm64 OCI 镜像。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35078647286) 在受保护环境审批后，核验签名、摘要、SBOM、Sigstore 来源及连续晋级证据，再公开版本并更新 testing 渠道。

匿名下载的版本及渠道签名清单与候选一致，SHA-256 为 `4f21e094485b4c8cbb1377dc4d59c4542adc66354f1f8fd8d5bf20747ff4b613`。Windows 安装器为 `37,782,429` 字节，SHA-256 为 `8da454bcf6ca107549979507512f2be8069de7afafb242ed23a67a40239ef231`。

## 生产与兼容

升级前备份 `20260916T084636914976Z-40ff8bf4` 通过校验。隔离恢复验证 agent_room、keycloak、synapse 三个数据库及 PITR 目标，用时 7.424 秒。候选迁移器执行成功。

先部署后端，实际安装的 Alpha 36 CLI 对 Alpha 37 服务器恢复同一已保存人物和目标房间，5 条未确认投递仍可读取；再升级本机应用；公开发行与渠道验证通过后部署网页并推进保存的下载入口。数据库、Matrix 和对象存储容器保持原样，仅应用镜像引用变化。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:5a13487e86833d969910a73659cd7b828e7603394cf84744a8a57ab28c76f1f5` |
| identity | `sha256:bcdf1fa255705e82f1b07499665aba893e0673424eea43be26c88823734a373a` |
| web | `sha256:8682bdb2fbe21097841c9d6bea84505e2e823aacb057c3afbf04061814e49848` |

公网健康与联邦检查通过，API 返回 Alpha 37，依赖全部就绪并接受准确的桌面 Origin。

## 实机验收

三份验收均在本次签名候选上执行。本轮宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5；Codex 账号额度耗尽，不再作为本次宿主。

- `first-device`：隔离设备资料从签名 Bridge 完成设备授权，人物进入指定生产房间，回复经接收端验证。
- `upgrade`：实际安装的 Alpha 36 升级到 Alpha 37，安装器退出码 0，运行时文件摘要与发布产物一致，登录、人物身份和 5 条未确认投递全部保留。
- `continuous-reception`：真实宿主处理两条不同消息并产生两条已验证回复；第二条带附件，宿主经 `agent_room_open_content` 取得下载路径后用只读文件工具读出验证码并在回复中给出，界面确认两条回复可见；71 秒空闲期间宿主调用次数保持为 2，未唤起模型；人工接管使后台接收进程退出且服务端转为 `idle`；交回后运行编号变化而游标不变，没有重复处理已回复的消息。

验收使用的临时人物已交回授权并退出房间，一小时限额的回复授权已撤销，隔离 Bridge 与接收进程已停止，本机桌面恢复普通启动且调试端口已关闭。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份与部署报告保存在 `/var/lib/agent-room/releases/alpha37/`；本地发布报告位于 `artifacts/releases/alpha37/`。

发布阶段为 `clients-published`；短期上线检查不代表长期兼容观察，未收缩旧协议。公开测试 Go/No-Go 仍为 NO-GO，开放阻断未变化。
