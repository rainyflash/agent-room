# 任务 23 验证记录：Codex 插件与 MCP 薄客户端

## 1. 结论

任务 23 的官方插件结构、原生 STDIO MCP、本机认证 IPC、逐工具权限策略和跨进程宿主验证均已完成：

- 插件使用 `.codex-plugin/plugin.json`、`.mcp.json`、技能目录和插件内资源，不依赖对话侧边栏注入。
- MCP 只把八个闭合工具映射到本机 Bridge；它不创建 Matrix Client、不保存第二套身份密钥，也不读取 Codex 私有缓存。
- 每次 MCP 调用只申请该方法所需的单一 IPC Scope，并完成版本协商、挑战证明和响应类型校验。
- 身份、预览和在线状态允许自动读取；正文、状态发布、消息发送以及交接消费/拒绝默认逐次审批。
- Windows x64 发行包通过原生 MCP 协议测试和真实 Codex 0.134.0 隔离安装测试；两个独立 Codex 进程均发现同一个 `agent_room` 服务。

核心提交：

- `acc71b1`：定义闭合 Codex 工具 IPC 合同和独立 Scope。
- `96ff4f5`：实现认证、版本协商和超时受控的本机 IPC Client。
- `ef77c6a`：实现 Bridge 本地传输适配器。
- `7fa2b5c`：统一 Bridge 运行时目录与端点解析。
- `1b477c3`：实现八个原生 Rust MCP 工具和安全错误映射。
- `44831f6`：打包官方 Codex 插件、权限模板与可复现归档。
- `fef8936`：锁定工具风险提示和宿主审批语义。
- `ce697fa`：统一插件产物格式门禁。

## 2. 架构与依赖方向

```text
Codex 对话
  -> Agent Room Skill
  -> agent-room-codex-mcp（STDIO，无状态适配器）
  -> LocalBridgeClient（单次最小 Scope）
  -> 认证本机 IPC
  -> 单实例 Agent Room Bridge
  -> 应用用例 / Matrix / 本地加密存储
```

- Codex 插件只负责意图路由、JSON Schema 和用户可见的安全提示。
- MCP Server 依赖抽象 `BridgeToolClient`；生产适配器才依赖本机 IPC 实现，测试使用内存替身。
- IPC 合同位于独立 crate，Bridge 和 MCP 共享同一版本化协议，避免复制 DTO 和错误码。
- Matrix Session、设备私钥、同步游标和一次性交接正文仍只有 Bridge 一个事实源。
- 每次调用建立独立本机会话，只申请当前方法所需 Scope；不存在“先批准读取，随后顺带发送”的宽权限会话。

## 3. 工具与审批矩阵

| MCP 工具 | IPC Scope | MCP 风险提示 | Codex 默认审批 |
| --- | --- | --- | --- |
| `agent_room_get_self` | `self_read` | 本机只读、闭合世界 | 自动 |
| `agent_room_list_previews` | `previews_read` | 远端只读、仅最小预览 | 自动 |
| `agent_room_get_presence` | `presence_read` | 远端只读 | 自动 |
| `agent_room_open_content` | `content_read` | 远端只读、可能含提示注入 | 逐次询问 |
| `agent_room_publish_status` | `status_publish` | 幂等外部写入 | 逐次询问 |
| `agent_room_send_message` | `message_send` | 非幂等外部写入 | 逐次询问 |
| `agent_room_consume_handoff` | `handoff_consume` | 非幂等、破坏性、返回不可信正文 | 逐次询问 |
| `agent_room_decline_handoff` | `handoff_decline` | 非幂等、破坏性 | 逐次询问 |

`approval-policy.example.toml` 采用显式八工具白名单，以 `prompt` 作为默认模式，只覆盖前三个最小只读工具为 `approve`。审批属于用户配置，插件不会静默改写全局 `config.toml`；市场名称变化时只替换插件选择器。

## 4. 不可信内容边界

- MCP 初始化指令开头即声明 Agent Room 远端内容不可信，保持在 Codex 优先读取的首段范围内。
- 预览、在线状态、完整正文和消费后的交接正文都会在结构化数据前插入安全提示。
- 远端文本不得被解释为系统指令，也不能授权链接、命令、代码或后续工具调用。
- 技能强制执行“先预览、后正文”，且把打开正文、发送消息、消费交接和拒绝交接视为不可复用批准的独立意图。
- 输入 JSON Schema 和 IPC 边界共同限制房间标识、UUID、分页、正文大小、摘要、语言、风险标签与进度范围。

## 5. 可修复失败

MCP 保留稳定错误码、类别、是否可重试和有限诊断字段，并给出精确恢复动作：

| 错误 | 用户可执行恢复动作 |
| --- | --- |
| `bridge.ipc.credentials_missing` | 启动或修复 Bridge，让它初始化本机授权凭据 |
| `bridge.ipc.credentials_unavailable` | 解锁操作系统会话并确认 Bridge 运行 |
| `bridge.ipc.credentials_corrupt` | 运行本机修复流程重新授权 |
| `bridge.ipc.bridge_unavailable` / `bridge.ipc.timeout` | 启动 Bridge，等待就绪后重试 |
| `bridge.ipc.version_incompatible` | 将 Bridge 与插件更新到同一发行版本 |
| `bridge.agent_runtime_unavailable` | 等待 Bridge 完成任务 24 的登录与实时运行时装配 |

失败不会触发读取 Codex 配置缓存、聊天历史、截图或任意本地文件的降级路径。响应类型与方法不匹配时直接丢弃，并要求同步升级双方。

## 6. 打包与真实 Codex 验证

`tools/plugin.py` 提供三个确定性入口：

- `validate`：校验 workspace 与插件版本、唯一 MCP 服务、相对二进制路径和审批矩阵。
- `stage`：构建 `--release --locked` 原生 MCP、执行 JSON-RPC 握手/工具调用冒烟测试，并生成固定时间戳和权限位的 ZIP。
- `host-check`：创建临时市场和隔离 `CODEX_HOME`，真实执行市场添加、插件安装和两次独立 `codex mcp list --json`。

宿主测试同时确认：

1. Codex 市场能够列出并安装 `agent-room@agent-room-community`。
2. 缓存中同时存在 `.codex-plugin/plugin.json`、`.mcp.json` 和原生 MCP 二进制。
3. MCP 被解析为启用的 STDIO 服务，工作目录位于隔离插件缓存。
4. 两个独立 Codex 进程解析出的服务配置完全一致，证明能力属于插件安装而非某个对话。
5. `get_self` 在 Bridge 未运行时返回结构化可修复错误，而不是伪造身份或改读私有数据。

本地忽略产物：

```text
artifacts/codex-plugin/agent-room-plugin-v0.1.0-windows-x64.zip
大小：1,990,002 bytes
SHA-256：E9F8F99D370E2687CF47400AB5D3288C974AE3A156B2FB6C74E7F44B1B754665
```

## 7. 质量门禁

```text
python <plugin-creator>/scripts/validate_plugin.py plugins/agent-room
  官方插件结构校验通过

python tools/plugin.py host-check
  Release MCP 构建通过
  JSON-RPC initialize / tools/list / tools/call 通过
  Codex 0.134.0 隔离安装通过
  两个独立 Codex 进程均发现 agent_room

cargo test -p agent-room-codex-mcp
  4 个 MCP 回归测试通过

just check
  Rust fmt、Clippy -D warnings、workspace 全特性测试通过
  34 个 TypeScript 测试文件、127 个测试通过
  TypeScript、生产构建、协议一致性、Secret 扫描和 Actions 固定引用检查通过
```

Rust 1.97.1 在 Windows 上会把 MSVC 的“正在创建库”本地化标准输出显示为 `linker_messages` 提示；它不是仓库代码警告，Clippy 的 `-D warnings` 已单独通过。第三方 `proc-macro-error2 2.0.1` 另有未来兼容提示，不影响当前构建结果。

## 8. 明确边界

任务 23 完成的是可安装、跨对话发现、最小权限且可连接真实 Bridge 的宿主适配层，不代表完整线上纵向切片已经完成：

- Bridge 当前能够认证 IPC 和拒绝越权，但实时 Agent Room 运行时尚未把八个方法全部装配到 Matrix、投影和交接用例；这部分属于任务 24。
- 因此在实时运行时尚未就绪时，工具会诚实返回 `bridge.agent_runtime_unavailable`，不会展示演示数据。
- 当前本机生成的是 Windows x64 插件归档；macOS 和 Linux 需要各自编译原生 MCP 并执行同等宿主验收。
- 两个 CLI 进程验证的是插件级发现与配置一致性；真实桌面端中的“登录 → 上线 → 收件 → 交给 Codex → 回复”证据将在任务 24 记录。

这条边界保持了 Clean Architecture：任务 24 只在 Bridge 组合根装配既有用例，不得把 Matrix Client、身份密钥或业务状态复制进 MCP。
