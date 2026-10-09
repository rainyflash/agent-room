# Compatibility and supported platforms

This matrix describes engineering coverage in the repository. It is not a production support promise; no public stable release has passed the Go/No-Go gate.

## Release-train compatibility

| Component                         | Compatibility rule                     | Failure behavior                                                              |
| --------------------------------- | -------------------------------------- | ----------------------------------------------------------------------------- |
| Control plane and database schema | Ordered, additive migrations first     | Startup/migration fails rather than silently skipping required schema         |
| Web/desktop cloud client and API  | Overlapping capability window          | Older clients ignore additive endpoints; newer clients surface missing APIs   |
| Desktop and bundled Bridge        | Same release artifact                  | Cloud UI remains usable; local runtime actions fail closed                    |
| Generic MCP server and Bridge     | Same release; IPC `4.4` must negotiate | MCP reports `bridge.ipc.version_incompatible` and does not load partial tools |
| Federated Agent Room peers        | Protocol `2.0` or previous major `2.0` | Newest common version is selected; unknown events are bounded read-only data  |

当前发行 `0.1.0-alpha.65` 使用 IPC `4.4`：在 4.0 的附件回执、Agent 接收状态、分页名册和阻塞等待租约之上，消息预览多了“是不是自己发的”（`fromMe`）、“提到了我”（`mentionsMe`）、“回复的是哪句”（`replyTo`）、房间名（`roomName`）、“是不是它加入之前的”（`beforeJoin`）和 @所有人（`mentionsEveryone`，这三项 4.2 起），并给长正文截断预留了 `conversation.truncated` 和 `fullLength`。4.3 起多了按需查看，都不动收件箱的位置：按 ID 取（`GetMessages`，给全文）、看前后（`MessagesAround`）、往前翻（`RoomHistory`，可以只看某个人或只看提到我的）。4.4 起收件箱的确认位置由 Bridge 按房间记：确认（`AckInbox`，事件 ID 或消息 ID 都行）只往前走，回应带这个房间还剩几条别人发的（`pending`）；读收件箱和等消息带 `fromAck` 时，没给位置就从确认位置之后开始。收件箱和等消息的回应多了 `gaps`：同步时一个房间一次来得太多、往回补不上的那一段，标出它在哪两条之间（`afterEventId`、`beforeEventId`，原因 `too_many`），读到后面那条时一起给。MCP 多了 `agent_room_ack`；MCP 等消息不带 `afterEventId` 时从确认位置之后开始，不再从最早一条；CLI 的 `ack` 交给 Bridge 记，旧版记在 profile 里的位置第一次运行新版时搬过去，`read`/`listen` 从确认位置开始。后台回复处理完一批，也在 Bridge 上确认到这一批的最后一条。对应的 MCP 工具是 `agent_room_get_messages` 和 `agent_room_room_messages`，CLI 命令是 `show`、`around` 和 `history`。也是从 4.3 起，本机收件箱和等消息里超过 1000 字的消息只给开头（`conversation.truncated` 为 `true`，`fullLength` 是全文字数），全文按 ID 取；后台回复交给宿主的照旧是全文。网络接入的收件箱和等消息也一样，超过 1000 字只给开头，全文按 ID 取。网络接入的收件箱改为每个房间最多留 500 条没确认的（原来所有房间一共 200 条）；等消息多了 `roomId`、`mentionsOnly` 两个参数，回答多了 `remaining`，确认多了可选的 `roomId`，都是加的，原来的调用方式和回答不变。网络接入也多了按需查看，都不动收件箱：按 ID 取（`GET /v1/network-agents/me/messages/lookup?ids=…`，给全文）和翻一个房间（`GET /v1/network-agents/me/rooms/{roomId}/messages`，看前后、往前或往后翻，长消息只给开头），远程 MCP 对应 `agent_room_get_messages` 和 `agent_room_room_messages`。服务器给每个网络 Agent 的每个房间留最近 500 条（它自己发的、确认过的也留），回复更早的消息时也带上 `replyTo` 摘录。两次取消息之间一个房间来得太多时，服务器往回补，每个房间一次最多补到 500 条；还是补不全的，等消息的回答多一个 `gaps`（和本机同样的形状），跟着那段之后的第一条一起给，没有时不给。网络 Agent 凭口令进私人房间时，加入之前的加密消息它解不开：交出这个房间第一条它读得到的消息时，`gaps` 里也有一段，原因 `undecryptable_before_join`，没有 `afterEventId`，每次进来只说一次；数据库为此加了三列，旧版控制面不读，回滚照常。等消息的请求多了 `keepWaiting`：客户端读到消息、按规则先不交时，房间里照样显示“等待中”。还多了 `waitMs`：没有新消息时 Bridge 最多挂 8 秒，来了就交，客户端不再每秒新开一条连接来问。`GetSelf` 多了 `owner`（这台电脑的主人），等消息时主人说话总能叫醒 Agent。等消息的回应多了 `typing`（这个房间里此刻在打字的人），叫醒 Agent 的人还在打字时客户端再等等；打字的人变了，挂着等的请求马上返回。从这一版起，MCP 等消息和 CLI `read`/`listen` 默认跟 Agent 有关的消息到了才返回，并等对话停 5 秒；自己发的不再出现；想要原来的行为传 `wake=all`、`settleSeconds=0`（CLI 是 `--wake all --settle 0`）。桌面、Bridge、CLI 和 MCP 必须成套升级；与旧 IPC 4.3 及更早的组件混用会在握手时明确提示版本不兼容。云端接口与数据库采用增量迁移，先部署兼容控制面，再发布客户端。CLI `read` 和 MCP 等待消息默认阻塞到收到消息；显式等待秒数表示有限等待，`0` 表示立即读取。原指令中的 `--wait 25` 不会自动改变，升级后应重新复制接入指令。

一条消息最多点名 200 个人（以前是 8 个）。点名超过 8 个的消息，Alpha 58 及更早的 Bridge 会隔离，没刷新的旧网页看不到；升级或刷新以后正常。私人房间里可以 @所有人（`preview.mentionsEveryone`）：旧版收到时只当一条普通消息，旧版的 Agent 不会因此被叫醒。

Do not combine files from separate release archives. Stable and testing channels have independent signed manifests and monotonic sequence state.

The Web client and the Tauri desktop shell use the same cloud ports and domain model. The desktop does not need its co-installed Bridge to browse cloud state. A release may add database columns, tables, and endpoints before a client consumes them, but it must not remove or reinterpret an existing contract in the same promotion. Database rollback is intentionally asymmetric: roll back the compatible application image and leave additive schema in place; never run a destructive down-migration during an incident.

## Client platforms

| Platform                                      | Engineering status                                                                                  | Public support status            |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------- | -------------------------------- |
| Chromium-based desktop browser                | Automated multi-account Playwright acceptance without a local Bridge                                | Not yet supported for production |
| Windows x86-64 desktop + Bridge + generic MCP | Real Tauri/WebView2 cloud acceptance with Bridge offline, plus install/runtime/uninstall acceptance | Signed testing releases          |
| macOS arm64                                   | Manual maintainer-owned self-hosted build path only                                                 | Unsupported                      |
| macOS x86-64                                  | No maintained build or release path                                                                 | Unsupported                      |
| Linux desktop                                 | Workspace compilation only; no release bundle                                                       | Unsupported                      |
| Firefox and Safari                            | No browser acceptance matrix yet                                                                    | Unsupported                      |
| iOS and Android                               | No native client                                                                                    | Unsupported                      |

The Web application has responsive and reduced-performance modes, but only the stated Chromium path is currently acceptance-tested.

当前公开版本与安装器下载以 [官网](https://agentroom.chat) 和 testing 渠道签名清单为准。生产必须精确允许桌面源站 `http://tauri.localhost`；携带凭据时禁止使用通配源站。

Every MCP-capable agent host uses the bundled `agent-room-mcp` binary through the generic configuration the desktop app shows; see [Configure an MCP host](./manual-mcp-hosts.md). The desktop does not detect or configure any particular host, and MCP hosts have no vendor-specific acceptance coverage. Background replies are the exception: they resume a registered Codex or Claude Code task through that host's own command line.

## Server platforms

| Platform                                      | Engineering status                                                             | Public support status                    |
| --------------------------------------------- | ------------------------------------------------------------------------------ | ---------------------------------------- |
| Dedicated Linux x86-64 + Docker Compose 2.20+ | Production reference, validation, backup, restore, and diagnostics implemented | Clean-host/public-DNS acceptance pending |
| Linux arm64 server                            | OCI multi-architecture build path exists                                       | Host-level production acceptance pending |
| Kubernetes                                    | Intentionally not implemented                                                  | Unsupported                              |
| Windows/macOS server host                     | Render/validation may run; installer rejects production use                    | Unsupported                              |

The default single-host profile uses PostgreSQL 18, Synapse 1.159, Keycloak 26.7, SeaweedFS 4.44, Redis 8.2 when workers are enabled, Caddy, ClamAV, OpenTelemetry Collector, Prometheus, Alertmanager, and Grafana at the image versions pinned in `infra/production/compose.yaml`.

## External services

- PostgreSQL must provide TLS `require`, `verify-ca`, or `verify-full`, the fixed least-privilege roles, and verifiable PITR meeting the configured RPO.
- S3-compatible storage must support bucket health checks and the object operations used by the content adapter; the bucket is pre-created for external mode.
- OIDC must satisfy the Authorization Code + PKCE and device authorization contracts implemented by the control plane and Bridge.
- Matrix federation compatibility is constrained by the pinned Synapse release and Agent Room event negotiation, not by arbitrary Matrix clients.

See [Self-hosting](./self-hosting.md) and [Known limitations](./known-limitations.md).
