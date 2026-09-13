# Alpha.27 发布记录

## 来源与范围

- 版本：`0.1.0-alpha.27`，testing 序号 `27`。
- 发布源码：`c4bc7a5abf107ee11d469f9fc3f9cf4ff001cfcf`（受保护 `main`）；与工作分支 `cb17e281aea7bb4de2d4f499e3d3d136aa6eeef5` 的文件树一致。
- [签名候选构建](https://github.com/rainyflash/agent-room/actions/runs/34735072947)采用 full 配置，包括 Windows 安装包、Desktop、Bridge、MCP、CLI、Codex 插件以及三个双架构服务端镜像。
- 变更包含统一的「接入 Agent」入口、任务接待、CLI / HTTP MCP、会话恢复和普通自动发言授权体验修复。最新代码的 CodeQL 代理路径与 TLS 配置问题已修复。

## 生产部署与兼容

2026-09-13 UTC 完成备份 `20260913T032629442410Z-675bf0eb`，完整性验证通过。隔离恢复在 6.867 秒内完成，三个数据库 `agent_room`、`keycloak`、`synapse` 的逻辑归档验证通过，PITR 到达指定恢复点。本版本没有数据库迁移或生产基础设施结构变更。

生产部署使用签名候选工作流产出的不可变镜像：

| 服务 | OCI digest |
| --- | --- |
| Control Plane | `sha256:67bd8b2b48fa77fd521be06a0675e7fd8eae3def1141320a298b0639200dc23f` |
| Identity | `sha256:5f02025db21f1b346255742ac41ba4f1d7621eb340075fad1c52c807839ed737` |
| Web | `sha256:98521b78278d2f3f276805e12f5534e7e0b2929aa44fbce46d0b4f99dfe051e8` |

三个容器健康检查、公开 HTTP / OIDC / Matrix 入口、联邦检查及 Web / `http://tauri.localhost` 精确 CORS 检查通过。旧版 Alpha.26 CLI / Bridge 在服务升级后恢复原 Agent、实例、Matrix 设备和房间，并成功读取正式房间消息。

真实浏览器在 PWA 更新后保留原账号和正式房间，显示真实 Agent 与新接入入口。生产入口资产为 `index-jH8vfC0m.js`。浏览器接入对话框正确引导到运行 Agent 的电脑及桌面安装入口；这项检查不代表已通过对话框接入真实宿主或完成后台唤醒。

部署记录和回滚镜像位于服务器 `/var/lib/agent-room/releases/alpha27/`，`/opt/agent-room` 源码也已检出该发布 SHA，保留原 Python 环境。备份与兼容服务证据由 `tools/release_promotion.py` 生成、绑定完整源码 SHA 并按顺序晋级。

## 验证边界

[完整 CI](https://github.com/rainyflash/agent-room/actions/runs/34735071081) 已通过：Rust / TypeScript / Python 检查、Windows 原生检查、供应链、Linux 运行时及 Matrix 恢复、浏览器回归、真实登录恢复，以及 PostgreSQL / Matrix / 对象存储集成。两个仅限自动回复专项分发的任务按工作流条件跳过，不计为本轮通过的检查。独立 [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/34735045622) 与 [联邦检查](https://github.com/rainyflash/agent-room/actions/runs/34735046282) 也通过。

同一源码的 Linux 运行时报告确认 CLI 为 Alpha.27、TLS 验证与 MCP 初始化成功，14 项工具可发现，错误令牌及越界 Origin 被拒绝。另一份隔离 Matrix 报告确认正常退出和崩溃后均能恢复 vault 与原身份，并完成真实消息投递；报告明确 `hostModelInvoked: false`。

真实模型回复和自动接待的既有边界见 [真实 Codex 验收](./live-codex.md)，不将协议、夹具或服务端测试冒充模型唤醒验收。

## 本机升级

在 `public-release` 环境固定 10 分钟等待期间，用 Alpha.26 已信任的 testing 公钥验证候选签名、Alpha.27 版本与序号 27，然后核对安装包摘要并覆盖安装到原路径。安装器退出码为 0，SHA-256 为 `baa080ec12229ab0aed6be153008fcb6cadc5ac0f4500b65a9eeee523244180f`。未修改环境等待规则。

Desktop 的 `--installer-version` 与 CLI 的 `--version` 均返回 Alpha.27；安装后的 Bridge、MCP、CLI 哈希逐件匹配签名清单。正常安装目录中的 MCP 完成真实 stdio 初始化，14 项工具及会话参数契约验证通过。

原 Agent `01a074a8-1baf-7731-8632-b94a46671f64`、实例与 Matrix 设备在升级后恢复到 `ready`，正式房间读取成功，无需为本轮验证重新登录。临时调试会话已关闭。此记录证明正常 Bridge 配置与 Agent 凭据恢复，不替代桌面每个页面的人工验收。

## 公开发布流程

[最终发布任务](https://github.com/rainyflash/agent-room/actions/runs/34736810397)的第一次尝试完成审核和 10 分钟保护等待后，持续超过 15 分钟未分配到 GitHub runner，未执行发布步骤。取消该排队尝试后，对同一任务做一次标准重试；源码、签名资产及部署证据均未变化，第二次尝试仍保留原有审核与等待规则。

第二次尝试在保护等待结束后仍未分配 runner。随后在本机执行未经修改的 `tools/release.py verify` 及 `tools/release_promotion.py verify` / `verify-evidence`：使用原已信任 testing 公钥、Cosign 3.1.3 和精确的 `main` 候选工作流身份，全部文件摘要、SBOM、Ed25519 / Sigstore 签名、序号与三个不可变 OCI 引用可达性均通过。停止第二次排队任务后，使用项目原有 `tools/release_surface.py` 完成发布，并更新 testing 渠道及客户端晋级记录。两个最终发布工作流尝试均为取消，不能记为 CI 发布成功；人工恢复的验证记录已作为发行资产保存。

[Alpha.27](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.27) 于 **2026-09-13 04:43:10 UTC** 公开为 prerelease。匿名访问 Windows 安装包返回 HTTP 200，长度为 36,982,235 字节；公开 testing 渠道清单与已完整验证的序号 27 签名清单完全一致。本机已安装的正是该发行包。发布状态现为 `clients-published`，尚未宣称完成长期兼容观察。
