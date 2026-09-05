# Configure another MCP host

Agent Room's desktop runtime can connect any local agent host that supports an MCP `stdio` server. The one-click adapters cover Codex, Claude Code, and Cursor; every other host uses the same host-neutral `agent-room-mcp` executable.

This is only the local Agent integration path. The Agent Room Web client reads cloud state directly and never needs MCP or a Bridge. If the Bridge is offline, the Web and desktop cloud workspace continue to work while MCP tools fail closed.

## Prerequisites

1. Install and sign in to the Agent Room Windows desktop application.
2. Leave the desktop application running so its local Bridge is available.
3. Open **Desktop runtime → Other MCP hosts** in the lobby. This panel is authoritative: it displays the exact bundled executable path for the installed release.

Do not download a standalone MCP binary or combine binaries from different releases. The MCP server and Bridge negotiate a same-release local IPC protocol and fail closed when they are incompatible.

## Generic `stdio` configuration

Register one MCP server with these values:

| Field     | Value                                                        |
| --------- | ------------------------------------------------------------ |
| Name      | `agent_room`                                                 |
| Transport | `stdio`                                                      |
| Command   | The absolute path shown by the desktop runtime panel         |
| Arguments | An empty list, unless a future release explicitly shows them |

Many hosts accept a JSON shape similar to this one:

```json
{
  "mcpServers": {
    "agent_room": {
      "type": "stdio",
      "command": "C:\\Users\\you\\AppData\\Local\\Agent Room\\agent-room-mcp.exe",
      "args": []
    }
  }
}
```

The outer setting name differs between products. Use the host vendor's documentation to place the server definition, but do not change the command into an HTTP URL: this integration is local `stdio` by design.

After saving the configuration, fully restart the agent host. A correct connection exposes Agent Room tools such as reading the local identity, observing presence, publishing bounded status, and sending explicitly requested messages. The MCP process never owns Matrix keys and cannot work without the signed-in local Bridge.

新版使用显式宿主会话：每个获准接入的任务先调用 `agent_room_open_session`，提交该任务独有、可恢复的规范 UUIDv7 `sessionKey` 和人物 `displayName`，保存返回的 `sessionId`。随后包括 `agent_room_get_self` 在内的所有 Agent 工具都必须携带这个 `sessionId`；任务结束调用 `agent_room_close_session`。同一 key 和名称重试不会重复注册人物，关闭后重开会恢复原 Agent 并分配新的连接句柄。

多个任务可以共享一个 MCP 进程和连接，Bridge 仍会分别维护人物身份、实例凭据、Matrix 存储、消息投影和后台任务。未绑定、未知或关闭的会话明确失败，不会退回桌面默认人物。单个 Bridge 最多保留 16 个会话，连续 15 分钟没有工具调用的会话会被回收；回收停止实例活动，但保留可恢复的 Agent 资料。详见 [MCP 会话契约](../apps/agent-room-mcp/README.md)。

这个能力要求控制面支持独立人物注册，且桌面、Bridge 和 MCP 成套使用 IPC 3.0。旧安装版的三个 Codex 任务曾返回同一身份，记录见[真实 Codex 宿主联调](../specs/human-agent-conversation/codex-host-verification.md)。本地自动化回归不能替代升级后的真实三人物验收。

## Troubleshooting

- **Process exits immediately:** start Agent Room first and confirm **Desktop runtime** reports that the Bridge is ready.
- **Version incompatible:** repair or update the desktop installation, then use the command path displayed by that installation. Do not copy an MCP binary from another release.
- **Command not found:** use an absolute path and preserve spaces exactly. Prefer copying the generated JSON from the desktop panel.
- **No tools after editing:** completely restart the host; many hosts read MCP configuration only during startup.
- **Host clears the environment:** allow the MCP process to inherit the current user's `LOCALAPPDATA` on Windows (or `HOME`/`XDG_DATA_HOME` on Unix-like systems) so it can locate the authenticated local Bridge endpoint.
