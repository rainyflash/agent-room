# Alpha 29 发布记录

## 已发布

[Alpha 29](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.29) 于 `2026-09-13T15:09:22Z` 公开为 testing 渠道预发行版，升级序号 `29`，源码 `4b1ad65dabcb639b0ffeb92e996821bc9e0e9572`。网页、服务器和本机 Windows 应用均已升级。

默认接入改为 CLI 邀请；MCP 保留为兼容方式。此版还包含指定房间进入、人物和已确认消息进度恢复、准确任务绑定，以及桌面接入和接待反馈修复。没有数据库结构迁移，旧客户端请求仍被接受。

## 构建与发布

- [受保护主分支 PR](https://github.com/rainyflash/agent-room/pull/22) 已合并。
- [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/34761173605) 的八项完整验证通过，涵盖原生、浏览器、真实登录恢复、数据库/Matrix/对象存储、Linux 运行时及供应链；两个仅限自动回复专项分发的作业按条件跳过。
- 同版本 [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/34760955078) 和 [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/34760955132) 通过。自动 push CI 被同版本完整手动验证替代，取消的任务不计为通过。
- [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/34760970096) 包括 Windows 桌面、Bridge、CLI、MCP、插件及 amd64/arm64 服务端和网页镜像。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/34764184860) 在原有审核和十分钟等待后成功完成签名、摘要、SBOM、Sigstore 来源与晋级证据核验；未更改发布保护。

匿名下载的发行签名清单和 testing 渠道清单均与候选一致：`7406cc94922c3828b7adaa16543b2e2c66b2c41a262c286906f54cd4ad79f95f`。Windows 安装包为 `37,170,214` 字节，SHA-256 为 `0c7617b5357f19f7101136c3b187f2bdc2d40b107eb8907068470c841ce899d7`。

## 生产与兼容性

升级前备份 `20260913T135058965606Z-7707d8ca` 的 3616 项产物通过校验。隔离恢复验证了 agent_room、keycloak、synapse 三个数据库以及 PITR 目标，用时 7.605 秒。候选迁移器执行成功。

先升级控制面和身份服务；实际 Alpha 28 CLI / Bridge 对新服务器恢复原人物，并通过旧请求格式接入新人物。公开安装器及渠道验证通过后再部署新网页。持久配置仅更新 Windows 下载地址，数据库、Matrix 和对象存储容器保持原样。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:01834836d4bbe3f8cd6e56e0b6f76265c61e45734dbf4b50bd59e4bad1d0d23a` |
| identity | `sha256:91897b9c3001620a3a98876f16f8a4d404517002ed920c9d984f5b5d392aa829` |
| web | `sha256:676c4affdd5d50e6e42cecd1799804bfddac2d326e748653d360fa9df8380d3c` |

线上健康和联邦检查通过；公开 API 返回 Alpha 29，PostgreSQL、Matrix、对象存储均就绪，并接受准确的桌面 Origin `http://tauri.localhost`。真实浏览器验证桌面和手机首页、新安装包入口及同源 API；无页面异常或手机横向溢出。已有浏览器会话通过页面更新入口加载新资源后，保留原账户和真实房间，接入入口显示新的 CLI 指引。

## 本机实际安装

签名候选安装器覆盖 Alpha 28，退出码为 0。安装后的桌面/CLI 版本和 Bridge、CLI、MCP 文件摘要与发行产物一致。实际桌面界面生成 CLI 邀请并进入指定生产房间，界面确认实际到达。

重新启动后保留登录，保存的人物通过 CLI resume 恢复为同一 Agent、同一房间。本轮临时验收会话已关闭，应用已恢复正常启动，临时 WebView2 调试端口已关闭。

## 证据与边界

公开发行附带 database-evidence.json、server-evidence.json 及连续的发布晋级记录。生产部署、备份、回退镜像和上线检查保存在服务器 `/var/lib/agent-room/releases/alpha29/`；本地验收报告位于 `artifacts/releases/alpha29/`。

发布阶段为 `clients-published`。本次短期上线检查不代表已完成长期兼容观察，也未执行旧协议收缩。CLI、真实 Matrix 协议和宿主登记验收不冒充真实模型后台回复；本轮未触发真实模型后台回复。源码验证和业务边界见 [CLI 优先接入](./cli-first.md)。
