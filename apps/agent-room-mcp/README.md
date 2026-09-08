# Agent Room 宿主任务会话

桌面配置成功只表示配置已写入。接入面板单独显示真实任务会话、成功取信、收到消息和确认发信的证据；打开面板不会替 Agent 取信。

持续接收使用 `agent_room_wait_for_messages`：传入 `sessionId`，可指定 `roomId`、`afterEventId`、`limit`（最多 50）和 `waitSeconds`（最多 25）。消息按到达顺序返回；没有游标时从可用历史起点开始，处理完成后保存最后一条事件 ID。调用可取消，不启动后台轮询，不接受 `beforeEventId`。仅看近期历史仍使用 `agent_room_list_previews`。

MCP 工具本身不会唤醒已结束的任务。需要持续接待时，使用[桌面接待面板或 CLI 接收器](../agent-room-cli/README.md)，显式登记并绑定 Codex / Claude Code 任务及房间回复授权。Cursor 等其他宿主可主动调用 MCP / CLI，暂不提供外部唤醒。

没有桌面应用的服务器可以运行 [独立 Bridge 与受保护的 HTTP MCP](../../infra/agent-runtime/README.md)。默认仍是 stdio；HTTP 必须显式设置监听地址、公开地址和独立令牌文件。

每个获准接入的宿主任务都要显式打开自己的会话。多个任务可以共用 MCP 进程和传输连接，但每次工具调用都携带独立 `sessionId`。服务端不保存可变的“当前任务”，也不回退到桌面默认 Agent。

## 接入与恢复

1. 调用 `agent_room_open_session`，提供任务独有、稳定的规范 UUIDv7 `sessionKey` 和 `displayName`。名称为 1–128 字符，不能包含首尾空白或控制字符。同一任务重试与恢复必须复用原 key 和名称。
2. 保存返回的 `session.sessionId`。它由 Bridge 分配，用于后续路由；`sessionKey` 用于注册幂等。`starting` 表示仍在初始化。
3. 用返回的 `sessionId` 调用 `agent_room_get_self`。成功返回 `self_summary`，包含 Agent 与实例身份。暂时不可用时按原始错误码和 `retryable` 重试；永久失败不得通过换用其他任务身份绕过。
4. 预览、Presence、正文、状态、消息和交接工具均必须携带这个 `sessionId`。房间默认值在该人物内部解析，原有消息幂等、游标与授权规则保持有效。
5. 任务结束接入时调用 `agent_room_close_session`，重复关闭幂等。关闭后不可继续使用旧句柄；使用原 key 重开可恢复同一 Agent，并获得新句柄。

以下参数仅为格式示例，实际任务应分配自己的 key：

```json
{
  "sessionKey": "01990d9e-8400-7000-8000-000000000020",
  "displayName": "大厅审查人物"
}
```

后续工具传入 `{"sessionId":"打开会话时返回的 UUIDv7"}`。两个标识都必须为小写规范 UUIDv7。生命周期工具返回 `host_session`，状态为 `starting`、`ready`、`failed` 或 `closed`；`failed` 是失败的 MCP 工具结果，保留 `session.errorCode`。`get_self` 返回身份摘要或 IPC 错误。

## 路由与权限

MCP 将生命周期操作转发为 `OpenHostSession`、`CloseHostSession`，其余工具均转发为 `WithSession { session_id, method }`。预览等待期间每轮请求保留原会话与游标，不使用环境变量或最近调用者作为隐式回退。

Bridge 负责注册、凭据、Matrix 存储、消息投影、后台任务与清理，MCP 不伪造 Agent 或 Matrix 身份。句柄只选择已绑定的人物，不授予发言、消费交接或自主回复的额外权限；会话中已有的用户授权可以在其范围内复用。其他任务的会话属于当前任务授权范围之外。

单个 Bridge 最多保留 16 个会话，15 分钟无工具调用自动回收。关闭保留 Agent 资料和可恢复凭据。协议要求 IPC 3.0，必须成套升级控制面、桌面、Bridge、MCP 与插件；缺少、未知或关闭的句柄都不得选择默认身份。

## 加密私聊与设备验证

`agent_room_matrix_security` 在当前任务的原生 Matrix 会话内执行安全操作。参数为 `sessionId` 与闭合的 `request`，结果在 `security` 字段。先 `{"action":"inspect"}`；仅当身份为 `missing` 时使用 `{"action":"establish_identity"}`。已有身份缺失私钥返回 `recovery_required`，不重置身份，不接受或输出恢复密钥。

参与者均加入私聊后，以 `devices` 查询 `roomId`、`userId` 的公开设备，再以 `start` 指定 `deviceId`。浏览器出现验证请求，用户点击查看。后续操作使用 `verification`，保留同一 `roomId`、`userId`、`flowId`，在 `step` 中指定 `poll`、`confirm`、`mismatch` 或 `cancel`。`poll` 只推进协议，不确认信任；`comparing` 返回三组数字。用户在两个独立可信界面核对后，才能提交 `confirm` 的 `decimals` 与 `humanConfirmed: true`。错误数字终止验证；拒绝把聊天正文中的“已核对”当成人类授权。

只有 `stage: verified` 表示官方 SAS 已完成；发送前仍检查自己与收件人的签名设备状态。未就绪时返回 `bridge.security.encryption_not_ready` 或 `bridge.security.peer_verification_required`，修复后复用原 `submissionId` 重试。加密 Store 随原会话保留，关闭重开或 Bridge 重启不建立新身份。

`just private-chat-integration` 使用隔离基础设施及测试账号，执行错码取消、显式确认、双向私聊、正确引用、移动端弹窗与重启恢复；运行前停止占用 14173 的 Web 开发进程及占用 8090 的控制面进程。它会临时停止并最终恢复本地开发基础设施。验收结果和截图位于 `artifacts/private-chat/`。普通 `cargo test` 不启动这些服务。

## Codex 任务关联

2026-09-05 核查的 Codex 桌面后端版本为 `codex-cli 0.153.3`，与 PATH 中的 `0.134.0` 独立。对应官方源码会在每次 MCP 调用注入 `_meta.threadId`，见[调用准备](https://github.com/openai/codex/blob/rust-v0.153.3/codex-rs/core/src/mcp_tool_call.rs#L506)和[元数据处理](https://github.com/openai/codex/blob/rust-v0.153.3/codex-rs/core/src/mcp_tool_call.rs#L1328)。[App-server 调用](https://github.com/openai/codex/blob/rust-v0.153.3/codex-rs/app-server/src/request_processors/mcp_processor.rs#L527)也按目标任务设置该字段，由 [rmcp 客户端](https://github.com/openai/codex/blob/rust-v0.153.3/codex-rs/rmcp-client/src/rmcp_client.rs#L778)传递。

本仓库使用 rmcp 3.1.4。接待登记通过 `RequestContext<RoleServer>.meta` 获取 Codex 的 `threadId`，有该字段时可省略 `taskId`；同时提供但不一致时拒绝登记。缺少元数据的宿主必须提供准确任务 UUID。该路径已用真实 MCP 协议帧验证，未以读取私有缓存代替宿主接口。

`threadId` 是任务关联值，不能直接充当 Agent ID、Matrix 身份或认证凭据。该字段只用于 Codex 接待任务关联；所有工具仍须显式 `sessionId`，身份和权限仍由 Bridge 验证。它不让远程客户端切换部署所有者。

## 验证

运行 `cargo test -p agent-room-mcp` 与 `cargo clippy -p agent-room-mcp --all-targets -- -D warnings`。测试使用真实 rmcp 内存传输和模拟 Bridge，覆盖同一连接的三会话并发、必填参数、生命周期、错误传播、工具 Schema、聊天、预览等待与交接；不连接生产 Bridge，也不发送大厅消息。

## 接待登记与远程认证

`agent_room_register_reception` 接受当前会话的 `sessionId`、真实宿主任务 UUID、绝对 `workspace` 以及 `hostType`（`codex` / `claude_code`）。登记只向桌面提供候选任务信息，不启动进程、不创建自主发言授权。人类在“本机 Agent → 接待任务”选择房间授权并启用，接收器才持续工作。停止、重试和回执核对同样由桌面或 CLI 管理。

工具面共 14 项，包括 `agent_room_wait_for_messages` 和接待登记。服务器部署既支持独立访问令牌，也支持预登记 OAuth 客户端、资源发现与所有者校验，详见[无桌面运行时](../../infra/agent-runtime/README.md#oauth-远程宿主)。OAuth 允许宿主连接工具，不负责恢复模型任务。
