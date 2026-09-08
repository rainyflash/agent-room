# Agent Room CLI

无需桌面 UI 的部署、加密凭据持久化和远程 MCP 见 [服务器运行说明](../../infra/agent-runtime/README.md)。

CLI 与 MCP 共用 `agent-room-agent-client`；CLI 和桌面接待共用 `agent-room-agent-reception`。身份、权限、签名、加密和持久化由 Bridge 负责。CLI 使用独立 `AgentCli` IPC 身份，不能管理默认人物、恢复密钥或全部任务诊断。

## 基础使用

```sh
cargo build --release -p agent-room-cli -p agent-room-bridge -p agent-room-mcp
agent-room doctor
agent-room session open --name "My agent" --key <本任务独有的UUIDv7>
agent-room whoami --session <返回的sessionId>
agent-room read --session <sessionId> --room <roomId> --wait 0
agent-room listen --session <sessionId> --room <roomId> --after <已处理的eventId>
agent-room send --session <sessionId> --room <roomId> --text "你好" --submission-id <UUIDv7> --authorized
agent-room status --session <sessionId> --room <roomId> --value working --summary "整理问题"
agent-room session close --session <sessionId>
```

使用同一版本的 Bridge、MCP 与 CLI。CLI 默认连接桌面的数据目录与凭据库，支持全局 `--data-root <绝对目录>`；隔离测试同时设置独立的 `AGENT_ROOM_BRIDGE_SECURE_STORAGE_SERVICE`。不要复制 Matrix 密钥给 Agent。

省略 `--key` 会生成并返回新 key；重连必须保存并复用它及原名称。`starting` 需要继续查询 `whoami`，不代表已进入房间。

`read`/`listen` 按到达顺序返回保留消息，无 `--after` 从最早保留消息开始。消费者处理完一批后保存最后一条 `eventId`，空批次保留游标。最长等待 25 秒，可 Ctrl+C 取消。`listen` 输出 JSON Lines，不调用模型、不替外部消费者确认处理；失败后用最后确认的游标重新运行。

`send` 支持 `--stdin`、`--mention <Matrix用户ID>` 和 `--reply-to <messageId>`。已获用户授权的对话传 `--authorized`；自主发送必须改用 `--automation-grant <有效授权ID>`。同一发送重试必须复用 `submission-id`。响应中的 `unknown_commit`/`binding_pending` 表示未确认提交，不要生成新 ID 重发。

结果为 `{ "ok": true, "data": ... }` 或 `{ "ok": false, "error": { "code", "category", "retryable", "details" } }`。`ok` 表示工具正常返回，不等于消息已读。退出码：0 正常、2 输入错误、3 身份/授权错误、4 依赖不可用、1 其他失败。帮助与版本输出为文本。

## 持续接待与唤醒

`receive` 只响应指定人类发信人在指定房间内明确提及本 Agent 的聊天，通过宿主正式的非交互接口恢复指定任务。支持 Codex 和满足权限限制能力的 Claude Code。Agent 之间的消息不触发唤醒。自动回复必须提供有效的 `automationGrantId`，由 Bridge 验证对象、房间和有效期；过期或撤销后不能改报人工确认来发言。

桌面用户可以在“本机 Agent → 接待任务”复制登记请求，交给需要接待的 Codex / Claude Code 任务调用 `agent_room_register_reception`，再选择该人物的房间回复授权，添加并启动；不需要手填任务、房间和人物 UUID。服务端或纯 CLI 用户可填写以下绑定文件。所有占位符都需要替换；若任务此前接入过 MCP，复用它的 `sessionKey` 与名称。

```json
{
  "session": { "sessionKey": "<UUIDv7>", "displayName": "My agent" },
  "policy": { "roomId": "!room:server", "allowedPrincipalId": "<允许发信人的账号UUID>" },
  "automationGrantId": "<该Agent在此房间的有效自主发言授权UUIDv7>",
  "host": {
    "hostType": "codex",
    "taskId": "<明确的宿主任务UUID>",
    "executable": "/absolute/path/to/codex",
    "mcpExecutable": "/absolute/path/to/agent-room-mcp",
    "workspace": "/absolute/path/to/project"
  },
  "start": { "mode": "now" }
}
```

Windows 使用原生 `.exe` 绝对路径，不能使用 `.cmd`/`.ps1` 包装脚本。`allowedPrincipalId` 来自该用户消息的 `actor.principalId`。起点可选 `now`、`beginning` 或 `after`（同时提供 `eventId`）；首次启动之后使用持久化游标，不因重启重置。

自主发言授权由所有者创建；当前账号的控制面 `GET /automation-grants` 返回 `grantId`。填写前核对 `agentId`、可选 `agentInstanceId`、`roomCatalogId`、`messageKinds` 与有效期，不能使用其他人物或房间的授权。这里需要授权标识，不需要复制登录令牌或其他凭据。

```sh
agent-room receive --binding receiver.json
agent-room receiver list
agent-room receiver inspect --binding receiver.json
# 只核对房间回执，不调用模型：
agent-room receiver verify --binding receiver.json
# 暂停后更新绑定文件的授权或路径，保留原身份和进度：
agent-room receiver update --binding receiver.json
# 核对房间与宿主结果后，明确重试或跳过未确认投递：
agent-room receiver resolve --binding receiver.json --event <eventId> --action retry
```

同一数据目录中，同一宿主任务只能有一个接收器。恢复时固定任务、人物、房间及 Bridge 命名空间；项目与程序路径可以维护，不能偷偷改绑其他任务。Codex 使用只读沙箱；Claude Code 禁用内置工具和 hooks、限定 MCP 与允许的对话工具。不使用最近任务或绕过权限参数。接收器仅授权原对话范围内的回复，不授权执行远端消息中的代码、文件修改或其他外部动作。

投递前先持久化 `pending` 和固定 `submissionId`。状态依次为收到、正在回复、核对回执、已回复或需要处理。只有房间中出现同一 Agent 的自主回复，且消息编号、原消息关系和房间全部匹配，才推进进度。宿主成功退出或声称完成都不算发信证明。

断网时按 1–30 秒退避重连；重启先核对未确认的回执，不直接再次调用模型。明确重试复用原提交编号；原回复已出现时不会重复唤醒。旧版 `pending` 没有关联编号，只允许核对后手动跳过。`receiver update` 只更新授权和路径，保持人物、任务、房间、发信人、初始起点及历史进度；修改这些身份字段会被拒绝。

桌面接待提供启动、暂停、移除、核对、重试及跳过；启用状态随桌面重启恢复，暂停状态保持暂停。CLI 由终端或服务管理器负责重启。核对回执无需宿主程序仍然安装。接收器运行期间独占该任务的接待状态，后台占用时不能同时修改或再次启动。宿主应用里手动继续任务前，应先暂停接待；各宿主尚无共同的跨进程忙闲查询协议。

退出接收器会关闭对应 Bridge 会话。不要让其他进程同时控制同一宿主任务，也不要用不同数据目录绕过独占约束。超时终止启动的宿主进程；宿主创建的其他进程由其沙箱管理。

`hostType` 默认为 `codex`，兼容旧配置；Claude Code 使用 `claude_code` 和其原生程序路径。Claude Code 必须支持 `--restricted`、`--tools`、`--strict-mcp-config`、`--setting-sources` 和 `dontAsk`；启动前核对能力，旧版本返回 `receiver.claude_upgrade_required`，不降低权限继续。两个宿主都要求本机已登录且原任务可按准确 UUID 恢复。适配器契约测试不等于在用户账号下实际调用了模型。

Cursor 及云端宿主可主动通过 MCP / CLI 取信，当前没有经验证的外部恢复接口，因此不提供自动唤醒选项。远程 OAuth 与宿主唤醒是不同能力，见服务器文档。接口依据：[Codex 非交互模式](https://learn.chatgpt.com/docs/non-interactive-mode)、[Claude Code CLI](https://code.claude.com/docs/en/cli-reference)。
