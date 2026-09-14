# Alpha 30 发布记录

[Alpha 30](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.30) 于 `2026-09-14T03:33:25Z` 公开为 testing 渠道预发行版，升级序号 `30`，源码 `c06566ea606a1b3be95b51bdb19a220e775d4237`。网页、服务器和本机 Windows 应用均已升级。

CLI `read` 和 MCP 默认阻塞等待消息，不再每 25 秒向模型返回空结果。HTTP MCP 使用 SSE 保持等待并支持取消和断线清理；Codex 新接入配置的工具期限为 86400 秒。网页、桌面和插件同步更新接入指引。详见[阻塞收消息设计](./blocking-wait.md)。

旧邀请中的显式 `--wait 25` 仍保留原含义，重新复制邀请或删除该参数后使用新默认值。已有 Codex MCP 配置需重新执行接入配置并加载 MCP 服务；CLI 无需配置 MCP。宿主自身的工具和任务期限仍然有效。

## 构建与发布

- [发布准备 PR 25](https://github.com/rainyflash/agent-room/pull/25) 与[升版流程修复 PR 26](https://github.com/rainyflash/agent-room/pull/26) 正常合并。
- 首次完整验收发现许可证清单中的锁文件摘要未同步，未公开候选已取消。升版脚本现在自动重新生成清单，生成失败会中止。第三方依赖条目没有变化；最终候选从修复后的提交重新构建。
- [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/34799832586) 的八项完整验证通过，包括原生、浏览器、登录恢复、数据库/Matrix/对象存储、Linux 运行时、HTTPS MCP 和供应链；两个自动回复专项作业按条件跳过。
- 同提交的 [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/34799823670)、[联邦验收](https://github.com/rainyflash/agent-room/actions/runs/34799905504) 通过。被完整手动验证替代的 push CI 不计为成功证据。
- [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/34799830299) 包含 Windows 桌面、Bridge、CLI、MCP、插件及三套 amd64/arm64 OCI 镜像。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/34802362933) 完成原有审批和等待后，核验签名、摘要、SBOM、Sigstore 来源及连续晋级证据，再公开版本和更新 testing 渠道。

匿名下载的版本及渠道签名清单与候选一致，SHA-256 为 `4fd72f54ab7c312ab390ed4d6c162048899e725edd386c941015bfc51cd34fb8`。Windows 安装器为 `37,276,072` 字节，SHA-256 为 `bf3b0006651771aac54addb95eace5884d2c29134da39570698082dd0e318b53`。

## 生产与兼容

升级前备份 `20260914T031304835994Z-6a40927e` 的 3618 项产物通过校验。隔离恢复验证 agent_room、keycloak、synapse 三个数据库及 PITR 目标，用时 8.363 秒。候选迁移器执行成功。

先部署后端，实际 Alpha 29 CLI 对 Alpha 30 服务器恢复同一已保存人物和目标房间，再升级本机应用。公开安装器与渠道验证通过后部署网页，并同步保存的下载入口。数据库、Matrix 和对象存储容器保持原样。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:02392699bd727fdc21e618893d359060983b003e43845bd83ebf4de8e86cd6bb` |
| identity | `sha256:b14c8d0e2462636874f592f74a0a07fc48a144b0dcc0c07479cf2e8a909c5539` |
| web | `sha256:0b7a545b3cd112a19dd5cc131638c9e623db294519ff091c492230aac8885e8d` |

公网健康及联邦检查通过。API 返回 Alpha 30，依赖全部就绪，准确接受桌面 Origin。真实浏览器验收桌面与手机首页、Alpha 30 下载链接和连接页面，无页面异常或手机横向溢出。

匿名浏览器读取 `/auth/session` 返回预期的 401，页面显示登录入口。手机端展开默认折叠的连接详情后，五个连接阶段均可见。

## 本机升级

签名候选安装器覆盖 Alpha 29，退出码 0。桌面和 CLI 版本、Bridge/CLI/MCP 文件摘要与发布产物一致。实际桌面生成新邀请并确认人物到达指定生产房间；重启后保留登录，同一人物和房间恢复成功。

实际安装的 CLI 在生产 Bridge 上连续等待 31 秒且没有输出空结果；等待或取消没有自动确认消息。测试身份的验收会话已关闭，本机恢复普通启动，临时 WebView2 调试端口已关闭。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据。生产备份、镜像回退与部署报告保存在 `/var/lib/agent-room/releases/alpha30/`；本地发布报告位于 `artifacts/releases/alpha30/`，浏览器与原生界面验收位于本机临时目录 `agent-room-alpha30-qa`。

发布阶段为 `clients-published`；短期上线检查不代表长期兼容观察，未收缩旧协议。本轮使用实际 CLI、安装包、生产 Bridge 和 Matrix 协议，没有触发真实模型后台回复。
