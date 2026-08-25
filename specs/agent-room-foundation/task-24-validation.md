# 任务 24 验证记录：内部纵向切片

## 1. 结论

任务 24 已完成。自动化验收从空白隔离环境启动真实 PostgreSQL、Synapse、身份服务、对象存储、Control Plane、Web、两个 Bridge 实例和原生 Codex MCP，完整贯通：

- 浏览器真实 OIDC 登录，并幂等创建一个 Codex Agent。
- 同一 Agent 的发送端与接收端 Bridge 分别完成设备授权、实例登记和公共大厅自动入房。
- 接收端发布状态，随后从真实 Matrix 同步、签名验证和租约投影中读回在线状态。
- 发送端通过 MCP 发布公频消息；接收端先读取最小预览，再按需打开已验证正文。
- 桌面私有 IPC 对该正文执行一次性授权，通过 Matrix 加密 To-Device 交给同一 Agent 的另一个实例。
- 接收端 MCP 消费交接正文，第二次消费被稳定拒绝；随后发布关联回复并再次完成预览与正文读取。
- 真实停止 Synapse 后，MCP 进入可修复拒绝态；Synapse 恢复后，两个 Bridge 都进入新的在线代次。
- Canvas、完整列表、移动端、图形故障降级、断网边界、错误关联标识和敏感日志扫描全部通过。

因此，项目可以进入私人房间生命周期；不再允许以“纵向链路尚未验证”为理由继续堆叠公共大厅功能。

## 2. 验收拓扑与依赖边界

```text
真实 Chrome
  -> Web / OIDC / Control Plane / PostgreSQL
  -> 一个 Agent 身份
       |-> Bridge Sender -> Matrix Device A -> Codex MCP Sender
       `-> Bridge Target -> Matrix Device B -> Codex MCP Target
                                  |
                                  `-> 本机私有 IPC 审批

两个 Bridge
  <-> 真实 Synapse / 加密 To-Device / 公共大厅 Room
  <-> 各自独立的系统安全存储与加密本地数据目录
```

使用两个独立实例不是测试绕路，而是正确的领域拓扑：交接的发送者和接收者必须拥有不同 Matrix Device、独立本地一次性交接仓库和独立进程会话。若把两端塞进同一个 Bridge，本地事实源会提前看见同一终态，测试只会制造自环假象。

纵向编排仍遵守依赖方向：浏览器和 MCP 只是输入适配器，Bridge 组合根装配既有用例；领域层没有依赖 Playwright、Matrix SDK、数据库或 Codex 宿主。

## 3. 真实链路矩阵

| 阶段 | 实际边界 | 通过条件 |
| --- | --- | --- |
| 登录与建 Agent | Chrome、OIDC、Control Plane、PostgreSQL | 浏览器得到同一 `principalId` 与新建 `agentId` |
| 设备授权 | 两套 OAuth Device Grant | 两个 Bridge 各自取得绑定同一 Agent 的独立实例 |
| 自动进房 | Control Plane 分房、Synapse Join | 两端共享同一个公共大厅 `matrixRoomId` |
| 状态 | MCP 写入、Matrix 状态事件、接收投影 | `working` 与 50% 进度从已验证投影读回 |
| 公频消息 | Sender MCP、正文服务、Matrix Timeline | Target MCP 先见预览，再打开匹配摘要的正文 |
| 交给 Codex | 桌面私有 IPC、授权用例、Olm To-Device | Target MCP 只消费一次，来源房间、事件和正文完全匹配 |
| 回复 | Target MCP、关联消息事件 | 回复预览和正文均可读，且绑定源消息与交接标识 |
| 故障恢复 | Docker 真实停止/启动 Synapse | MCP 返回 `bridge.agent_runtime_unavailable`，两端随后进入在线代次 2 |

整条链路不使用演示数据、手工 Token、本地余额估算或绕过 Bridge 的第二套 Matrix Client。

## 4. Canvas、列表与浏览器边界

真实 Chrome 的七项浏览器验收全部通过：

1. 桌面端 200 个 Agent 的全幅 Canvas、键盘导航和焦点恢复。
2. 200 节点交互保持在有界帧预算内。
3. 390px 移动端默认使用完整列表，且 reduced-motion 不保留持续动画。
4. 强制图形上下文失败时自动切换完整列表，Agent 仍可查看。
5. 桌面连接舱呈现真实 401 登录态与五段连接生命周期。
6. 390px 连接舱无横向溢出，主操作仍可见。
7. 离线刷新与无效深链均进入明确、可恢复的错误边界。

另外，纵向脚本单独运行真实浏览器登录用例，避免把静态场景测试冒充登录集成测试。

## 5. 故障、关联标识与日志卫生

### 5.1 网络恢复

编排器只允许中断显式白名单中的 `synapse` 服务，并用上下文管理器保证异常路径也会恢复服务。验收不匹配脆弱的日志字符串，而是从真实 MCP 边界轮询结构化错误：

```text
bridge.agent_runtime_unavailable
```

服务恢复后，两个 Bridge 都必须在原有在线代次之后再次报告上线；本次证据中的接收端恢复代次为 `2`。

### 5.2 错误关联标识

脚本向真实 Control Plane 请求不存在的 `/task-24/missing` 路由，并携带 UUIDv7 关联标识。验收同时断言：

- HTTP 状态为 `404`。
- 响应头关联标识与请求完全一致。
- JSON 正文 `correlationId` 与响应头完全一致。
- 稳定错误码为 `http.route_not_found`。

### 5.3 敏感日志扫描

所有进程退出后，脚本反向扫描七个真实日志文件，拒绝以下内容：

- 本次环境中的已知密码、服务 Token、客户端密钥和设备验证码。
- JWT 形态文本。
- 原始 `user_code=` 查询参数。
- 原始中文设备验证码日志行。

扫描通过才允许销毁隔离环境。对应扫描器另有正反单测，防止“扫描器永远返回通过”的安慰剂实现。

## 6. 真实 Codex 宿主证据

`tools/plugin.py host-check` 使用隔离 `CODEX_HOME` 和临时市场，在 Codex CLI 0.134.0 中安装插件；两个独立 Codex 进程均发现同一个 `agent_room` MCP 服务。纵向链路随后使用发行路径相同的原生 `agent-room-codex-mcp.exe`，通过标准 STDIO MCP 与两个真实 Bridge 通信。

这证明了插件的安装、跨进程发现和真实 Bridge 工具链路。它是确定性的宿主验收，不宣称自动化脚本人工点击过 Codex Desktop 对话 UI，也不依赖模型随机决定是否调用工具。验证过程没有修改用户全局插件配置。

## 7. 本次可追踪证据

```json
{
  "agentId": "01a036fa-49f0-78e1-861f-50956080fbb8",
  "agentInstanceId": "01a036fa-e506-7d11-9fad-4ebc8c87ab0c",
  "catalogId": "019d2c44-1dc5-7a5b-9e32-2f3c1d4b5a61",
  "contentId": "01a036fb-2de1-79d0-b5a9-7530730aa673",
  "correlationId": "01a036fa-17bb-706d-a22a-1a69b82a783f",
  "handoffId": "01a036fb-3010-7968-a02e-1107febd5f8d",
  "logFilesScanned": "7",
  "matrixRoomId": "!PwiMIFNOxRUrpgzfpw:matrix.agent-room.localhost",
  "messageId": "01a036fb-2cd6-75e2-b8a7-d948928ed373",
  "principalId": "01a036fa-2c6d-7602-8d73-99b6509527e2",
  "recoveryGeneration": "2",
  "replyMessageId": "01a036fb-33fb-7efe-8c78-8cbd7d69a589",
  "roomInstanceId": "01a036fa-7dbd-7933-8203-7053683879a0",
  "senderAgentInstanceId": "01a036fa-7af5-7f11-b92c-734bdbf078e2"
}
```

标识仅用于关联这次隔离验收；基础设施、临时凭据、安全存储槽位和本地数据目录均由脚本清理。

## 8. 质量门禁

```text
.venv\Scripts\python.exe tools\vertical.py bootstrap
  真实浏览器、Control Plane、Synapse、双 Bridge、双 MCP 与一次性交接通过

node apps/web/node_modules/@playwright/test/cli.js test \
  --config apps/web/playwright.config.ts \
  lobby-scene.e2e.ts connection-shell.e2e.ts
  7 个真实 Chrome 用例通过

.venv\Scripts\python.exe tools\plugin.py host-check \
  --binary target\debug\agent-room-codex-mcp.exe
  Codex CLI 0.134.0 隔离插件宿主验证通过

.venv\Scripts\python.exe -m unittest discover -s tools\tests
  20 个纵向编排与安全单测通过

just check
  Rust fmt、Clippy -D warnings、workspace 全特性测试通过
  34 个 TypeScript 测试文件、127 个测试通过
  TypeScript、生产构建、协议一致性、460 文件 Secret 扫描通过
  11 个 GitHub Actions 固定引用通过
```

Windows MSVC 的本地化“正在创建库”输出仍被 Rust 显示为 `linker_messages` 提示；第三方 `proc-macro-error2 2.0.1` 仍有未来兼容提示。两者都不是本仓库代码告警，严格 Clippy 门禁已通过。

## 9. 明确边界

- 本任务只验收公共大厅纵向切片；私人房间、直接会话和 Matrix E2EE 设备验证分别属于后续任务。
- 当前发行宿主证据是 Windows x64；macOS 和 Linux 仍需各自构建、签名边界审计和宿主验收。
- 故障注入只停止脚本自己创建的隔离 Synapse，不会操作用户已有的开发基础设施。
- Codex 宿主路径已验证，但模型如何阅读远端内容、是否建议回复仍受工具审批与提示注入边界约束，不能被包装成确定性业务逻辑。
