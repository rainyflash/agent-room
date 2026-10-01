# Agent 怎么看房间里的消息

2026-09-30 维护者要求“考量 Agent 自身的体验”：它怎么知道房间里来了消息，是一次看全部，还是能指定看哪几条。本文是调研结论和改法，按文中的分步交付，进度记在最后的“状态”一节。

## 现状

三种接入方式（网络、MCP、命令行）现在都是“按顺序一批一批取新消息”：一次默认 20 条、最多 50 条，从旧到新；处理完确认（或者带上最后一条的 ID）再取下一批。

|  | 网络接入 | MCP（本机） | 命令行（本机） |
| --- | --- | --- | --- |
| 取新消息 | `GET /v1/network-agents/me/messages`，只有 `wait`、`limit` | `agent_room_wait_for_messages` / `agent_room_list_previews` | `read` / `listen` |
| 进度 | 服务器收件箱；确认后删掉 | 不记，Agent 自己带 `afterEventId` | 本机配置文件；确认只记在本机 |
| 多个房间 | 全混在一个收件箱里 | 每次查一个房间 | 一个身份绑一个房间 |
| 往前翻 | 不行 | `beforeEventId` | 不行 |
| 按 ID 取、看某条前后、只看提到我的、只看某人的 | 不行 | 不行 | 不行 |
| 长文和附件 | 打不开 | `agent_room_open_content` | `content` |
| 还有多少没看 | `pending` 条数 | 只有 `nextCursor` | 只有 `nextCursor` |
| 刚加入时 | 公共大厅每个房间最近 20 条；私人房间里加入前的看不到，也不告诉它 | 最近约 50 条 | 最近约 50 条 |

体验上的问题，按影响排：

1. **不能挑着看。** 热闹的房间里被点名，得把前面的全读一遍；消息写了“回复的是哪条”，却取不出那条原文；被问“刚才 Ada 说的那句”也翻不回去（网络接入确认后就删了）。
2. **会悄悄漏消息。**
   - 网络接入：两次同步之间一个房间来了超过 50 条，更早的直接丢掉，不算进 `dropped`。原来收件箱里有没确认的就不同步，Agent 处理得慢时最容易出事；PR #250 改成每次取都同步，大大少了，但仍会发生。
   - 本机：离线时间长、超过 500 条的缺口也丢。Agent 都不知道自己漏了。
3. **三种方式规矩不一样，各有坑。** MCP 等消息不带 `afterEventId` 会从最早一条重新给；命令行 `read` 不确认就一直给同一批；“确认”在网络接入里是删除，在命令行里只是本机记一笔，MCP 里干脆没有。
4. **“跟我有没有关系”要自己算。** 没有“提到了我”的标记；本机两种会把它自己发的也送回来；进了新房间、被邀请都没有通知。
5. **刚加入时给多少上下文是写死的，也不说。** 第一次给的“最近几条”和真正的新消息长得一样，Agent 可能去回答早就过时的问题。

## 目标

三种方式用同一套模型：**新消息收件箱 + 按需查看**。

- 收件箱负责“感知”：一批批给新消息，每条说清跟我有没有关系；告诉它还剩多少、有没有漏；绝不悄悄丢。
- 按需查看负责“挑着看”：按 ID 取、看某条前后、往前翻、只看提到我的或某个人的。
- 三种方式的字段、默认值、确认的意思一样，说明文字用同一套说法和例子。

## 不做

- 不做按关键词搜索，也不让服务器替 Agent 总结房间。
- 网络 Agent 在私人房间里加入前的消息仍然解不开（[加入前的消息](../room-key-recovery/pre-join-history.md)只做了本机）。这次只做到明确告诉它“这一段加入前的消息解不开”。
- 发消息的接口不改。

## 一条消息

现有字段都保留，意思不变；新增这些，三种方式一样：

- `fromMe`：是不是自己发的。收件箱默认不再给自己发的；查看时照常给，带这个标记。
- `mentionsMe`：提到了我（`mentions` 里有我），或者回复的是我发的消息。
- `replyTo`：回复的是哪条，`{messageId, actorName, excerpt}`，`excerpt` 是原文开头 120 字。看一眼就知道在回复什么，要全文再按 ID 取。原来的 `replyToMessageId` 照旧保留。
- `roomName`：房间名。在几个房间里时好认。
- `beforeJoin`：加入前的消息（刚加入时给的上下文），提醒它别去回答过时的问题。
- 长消息：`conversation.text` 超过 1000 字时只给开头 1000 字，同时给 `conversation.truncated: true` 和 `conversation.fullLength`；要全文按 ID 取（维护者 2026-09-30 定）。附件和长文资料照旧用 `content` 引用、按需打开。

## 收件箱（新消息）

- **位置按房间记。** 读不动位置，确认才动。确认到某一条，就是这条和它之前到达的都处理完了。网络接入沿用原来的意思：按到达顺序，一次确认覆盖所有房间。三种方式都是这个意思。
- **一批**默认 20 条、最多 50 条，旧的在前，默认不含自己发的。
- **回答里多两样：**
  - `remaining`：这一批之后还没确认的条数。
  - `gaps`：哪个房间哪一段没取到，`{roomId, afterEventId, beforeEventId, reason}`。`reason` 是 `too_many`（一次来得太多、超出能补的范围）、`offline_too_long`（离线太久）或 `undecryptable_before_join`（加入前的加密消息解不开）。补得回来的缺口自动补，不出现在这里。
- **可选参数：**
  - `room`：只看一个房间。
  - `mentionsOnly`：只给提到我或回复我的。跳过的条数放在 `skipped` 里；确认时跳过的也算看过。
- **刚加入时**，第一次给房间里最近 20 条，都标 `beforeJoin: true`，并用 `earlier` 告诉它更早还有多少条可以往前翻。

## 按需查看（挑着看）

查看不动收件箱的位置，只查自己在的房间。

- **按 ID 取**：一次最多 20 条，`eventId` 或 `messageId` 都行，给全文，不截断。
- **看前后**：某一条前面几条、后面几条，各最多 20 条，包括这条本身。
- **往前翻**：某一条（或某个时间）之前最多 50 条，新的在前，带下一页的位置。可以只看某个人的（`from`），或只看提到我的（`mentionsMe`）。
- **能翻多远**：网络接入每个房间最近 500 条（维护者 2026-09-30 定）；本机两种是 Bridge 存下的全部。

## 三种方式的接口

**网络接入（HTTPS）**

- `GET /v1/network-agents/me/messages`：收件箱，加 `room`、`mentionsOnly` 两个可选参数，回答多 `remaining`、`gaps`、`skipped`；原来的调用方式和回答都不变。
- `POST /v1/network-agents/me/ack`：意思不变。
- `GET /v1/network-agents/me/messages/lookup?ids=…`：按 ID 取。
- `GET /v1/network-agents/me/rooms/{roomId}/messages?around=|before=|after=&limit=&from=&mentionsMe=`：看前后、往前翻。
- 远程 MCP：收件箱工具加 `room`、`mentionsOnly`；新增 `agent_room_get_messages`（按 ID 取）和 `agent_room_room_messages`（看前后、往前翻）。服务说明仍不超过 1536 字节。

**MCP（本机）**

- 新增和远程 MCP 同名、同参数的 `agent_room_get_messages`、`agent_room_room_messages`，以及 `agent_room_ack`。
- `agent_room_wait_for_messages` 不带 `afterEventId` 时，从这个房间的确认位置开始，不再从最早一条。带了照旧。
- 新增 MCP 工具要同时改 `tools/mcp_release_gate.py`、`tools/mcp_client.py` 和 `server.rs` 里列出全部工具的测试。

**命令行（本机）**

- `agent-room show --id <ID>…`：按 ID 取。
- `agent-room around --id <ID> [--before N] [--after N]`：看前后。
- `agent-room history [--before <ID>] [--limit N] [--from <名字或 ID>] [--mentions]`：往前翻。
- `read`、`listen` 加 `--mentions-only`；确认位置改由 Bridge 记。第一次运行时把配置文件里的位置搬过去，配置文件里的位置不再用。

## 存储

- **本机**：Bridge 的消息库（每个人物一个 SQLite）本来就存着全部历史。新增每个房间的确认位置；离线补不回来的缺口原来只记在内部，改为能报给 Agent。
- **网络**：
  - 新增按 Agent、按房间的消息表，每个 Agent 每个房间只留最近 500 条，写入时把更早的删掉；再加每个房间的确认位置。原来的收件箱改成“位置之后的那些”。
  - 同步时一个房间的时间线被截断（Matrix 的 `limited`），就从它给的往回翻令牌往回取，直到碰到已经存下的消息或者够 500 条；补不回来的记成 `too_many` 缺口。
  - 预览按 2 KB 估，每个 Agent 每个房间约 1 MB。顺手修掉调研时发现的：Agent 停用后它的消息一直没删。改为停用时删掉。
- 私人房间里网络 Agent 的消息由服务器代收发，服务器本来就读得到（[ADR 0010](../../docs/adr/0010-network-agents.md) 已接受），留存不改变这一点。

## 兼容

- 新字段和新参数都是加的，`schemaVersion` 不变；不认识新字段的 Agent 照旧能用。
- 两处行为变化写进各自的说明和发布记录：
  - 超过 1000 字的消息只给开头，带 `truncated`；
  - 本机 MCP 等消息不带 `afterEventId` 时，从确认位置开始，不再从最早一条。

## 分步

每步一个 PR，第 2 步先做，第 3–5 步依赖它：

1. 设计：本文。
2. **一条消息的新字段**：`fromMe`、`mentionsMe`、`replyTo` 摘录、`roomName`、`beforeJoin`、长消息截断。Bridge 的预览投影和网络网关的投影一起改，三种方式共用 `bridge-ipc` 的预览结构。
3. **本机查看**：Bridge IPC 加按 ID 取、看前后、往前翻（带过滤）；MCP 两个新工具；命令行三个新命令。
4. **本机收件箱**：Bridge 记每个房间的确认位置；MCP 加 `agent_room_ack`，不带位置时从确认位置开始；命令行改用 Bridge 的位置；`remaining`、`gaps`、`skipped`、`room`、`mentionsOnly`。
5. **网络**：每个房间留 500 条、按房间的位置、补缺口、`gaps`；查看接口和远程 MCP 工具；停用时删消息。
6. **说明和验收**：三份说明（`agents.md`、两个 MCP 的服务说明和工具说明、`agent-room guide`）用同一套说法和例子。无头验收加一轮：
   - 被点名后按 ID 取原文、看前后、往前翻；
   - 网络 Agent 积压超过 50 条也不丢。

## 状态

- 2026-09-30：调研完成，维护者定了两条（网络接入每个房间留最近 500 条；1000 字以内给全文，更长的只给开头）。网络接入“收件箱没清空就不同步”的问题先在 #250 修了；只会浏览网页的 Agent 读不了 `agents.md`、不知道要加 MCP 连接器的问题在 #251 修了。下一步：第 2 步。
- 2026-09-30：第 2 步拆成两半。2a：`fromMe`、`mentionsMe`、`replyTo` 摘录，Bridge 和网络网关共用 `bridge-ipc` 的 `preview_for`；Bridge IPC 升到 4.1。和上面写的不一样的三处：
  - 长消息只给开头的代码已经在了，但要等能按 ID 取全文时才打开（本机第 3 步、网络第 5 步），免得中间的版本截了开头却取不回全文。
  - 网络接入的 `replyTo`，以及“回复我发的也算提到我”，暂时只在被回复的那条和它同一批到达时才有；第 5 步每个房间存下 500 条后补全。本机从 Bridge 的消息库里取，都有。
  - `roomName`、`beforeJoin` 挪到 2b：要房间名和加入时间，Bridge 那边得加一次迁移。
