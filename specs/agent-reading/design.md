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

每步一个 PR，第 2 步先做，第 3–5 步依赖它。什么时候叫醒 Agent、叫醒时给什么，另写在 [Agent 怎么等消息](./waiting.md)；那是维护者最看重的，排在第 3–5 步之前做。

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
- 2026-10-01：2b 先做网络接入和公共部分。
  - 预览多了 `roomName`（房间名，不知道就不写）和 `beforeJoin`（是这个 Agent 加入房间之前的）。`preview_for` 多一个参数 `PreviewRoom`：房间名和加入时间。
  - 比较时用服务器收到消息的时间，没有才用发送方自己写的时间。不知道什么时候加入的就不标，宁可不标也不标错。
  - 网络接入：加入时间是控制面记下的 `network_agent_room.joined_at`，房间名在读房间记录时从房间目录关联出来。`joined_at` 是控制面在 Matrix 加入完成后才记的，这几毫秒到几秒里到的消息会被标成加入之前，影响很小。
  - Bridge IPC 升到 4.2：旧版组件不认这两个字段，握手时就说清版本不兼容。
  - 本机 Bridge 还没记房间名和加入时间（同步时把 Matrix 事件里的 `prev_content` 丢掉了，分不清“加入”和“改资料”）：下一步加一次迁移，只在亲眼看到自己加入时记下时间，离开时清掉。在那之前本机的预览里没有这两个字段。
- 2026-10-01：2b 本机 Bridge 这半。
  - 映射 Matrix 事件时留下上一个成员状态（`unsigned.prev_content.membership`），分清真正加入和改昵称、换头像。
  - 同步时挑出房间名和自己成员状态的变化（`bridge-core` 的 `room_state_changes`，纯函数），记进本地消息库的新表 `message_room_state`（迁移 0006）：
    - 成员状态从别的变成 `join` 时记下服务器收到加入事件的时间；
    - 离开或被移出时清掉，再加入时重记；
    - 房间名跟着 `m.room.name` 走。
  - 读消息时按房间读出名字和加入时间，交给 `preview_for`。
  - 升级前就在的房间：Bridge 每次上线的第一次同步带全量状态，房间名都能补上；自己最近一条成员事件就是加入事件的，加入时间也能补上；之后改过昵称、换过头像的分不清，就不标 `beforeJoin`。
- 2026-10-02：第 3 步拆成两半，3a 是 Bridge 和 IPC。
  - IPC 升到 4.3，多了三个方法：按 ID 取（`GetMessages`）、看前后（`MessagesAround`）、往前翻（`RoomHistory`）。回应是新的 `Messages`（带 `missing`、`more`）和 `RoomMessages`（带 `nextCursor`），不跟收件箱的 `MessagePreviews` 混用，诊断里也不算“读了收件箱”。旧 Bridge 不认这几个方法，握手时就说清版本不兼容。
  - 按 ID 取：事件 ID、消息 ID 都行，不用给房间；只给 Agent 所在房间里的，其余放进 `missing`。给全文，一次回复放不下的（一条就可能十几 KB）放进 `more`，再取一次。编辑过的消息只认原来那条的事件 ID。
  - 看前后、往前翻：按这台 Bridge 收到的先后排，和收件箱一样长消息只给开头。往前翻能只看某个人（Matrix 用户 ID，或者不分大小写的名字）、只看提到我的，判断“提到我”和读消息用同一个 `preview_for`；带过滤时一次最多看 500 条就先交，给接着翻的位置。
  - 和上面写的不一样：“某个时间之前”这次没做。本机按收到的先后排，离线补回来的历史排在后面，按时间翻容易让人误会；要的话以后再加。
  - 3b：MCP 两个新工具、命令行三个新命令，打开收件箱的长消息截断，改说明。
- 2026-10-02：3b（#284），MCP、命令行和收件箱截断。3a 是 #283。
  - MCP 加 `agent_room_get_messages`（按 ID 取）和 `agent_room_room_messages`（给 `around` 看前后，不给就往前翻，给 `after` 往后翻）。看前后时 `limit` 条前后各一半（前面多给一条，每边最多 20 条），另加它本身；`around` 不能和翻页、过滤一起给，参数错误报 `agent.messages.query_invalid`，不问 Bridge。
  - 命令行加 `show --id`（可以写多次）、`around --id [--before N] [--after N]`（默认各 10 条）、`history [--before|--after <ID>] [--limit N] [--from <名字或 ID>] [--mentions]`。
  - 本机收件箱和等消息打开长消息截断（超过 1000 字只给开头）。后台回复交给宿主之前按 ID 取回全文，宿主照旧读到整条；取不回来就给开头，宿主还能用只读工具去取。
  - 本机 MCP 的服务说明没改：已经 1522 字节，离上限 1536 只差一点，留到第 6 步三份说明一起重写；这次改了工具说明、两份 README 和 `agent-room guide`。
  - 下一步：第 4 步（本机收件箱：确认位置交给 Bridge 记）。

- 2026-10-02：第 4 步拆成三块，4a 是 Bridge 和 IPC。
  - IPC 升到 4.4，多了确认（`AckInbox`）：给事件 ID 或消息 ID，不用给房间，Bridge 自己找到它在哪个房间，只认 Agent 所在的房间。位置记在本地消息库的新表 `message_inbox_ack`（迁移 0007），每个房间一行，只往前走：往回确认不动位置，回 `acknowledged: false`。回应带这个房间确认位置之后还有几条别人发的（`pending`），和网络接入的确认同一个说法。收到以后才撤回的那条也能确认到它。
  - 读收件箱、等消息的请求多了 `fromAck`：没给 `afterEventId` 时从确认位置之后开始，没确认过就从最早一条开始。不带它的照旧从最早一条开始，后台回复用的是服务器上的进度，不受影响。
  - 和网络接入不一样的一处：找不到的、不在所在房间里的消息，确认时报 `bridge.message_not_found`，不回 `false`。本机分得清“早就确认过了”和“没有这一条”，报出来 Agent 才知道自己给错了。
  - 4b：MCP 的 `agent_room_ack`，等消息不带位置时从确认位置开始；命令行改用 Bridge 的位置，第一次运行把配置文件里的位置搬过去；`mentionsOnly`。4c：缺口（`gaps`）。
- 2026-10-02：4b，MCP 和命令行用上 Bridge 记的确认位置。
  - MCP 加 `agent_room_ack`（`sessionId`、`eventId`，和远程 MCP 同名同参数），回 `acknowledged`、`pending`；等消息不带 `afterEventId` 时从确认位置之后开始。说明里改成“处理完一批用 `agent_room_ack` 确认到返回的 `nextCursor`”：`nextCursor` 是交出去的最后一条，跳过的也算看过。
  - 命令行的 `ack` 交给 Bridge，`read`、`listen` 从确认位置开始（给了 `--after` 就以它为准）。旧版记在 profile 里的位置（`afterEventId`）第一次连上时确认到 Bridge，之后 profile 里不再记；那一条本机消息库里没有时，就当没确认过。原来“只能确认交付过的消息”的限制去掉了：Bridge 只认这个 Agent 所在房间里的消息。
  - 和上面写的不一样，多做了一处：后台回复处理完一批（回了或者不用回），也在 Bridge 上确认到这一批的最后一条。不然同一个人物之后用 MCP 或命令行不带位置等消息，会把后台回复处理过的再收一遍。
  - `mentionsOnly` 挪到 4c，缺口（`gaps`）挪到 4d。
- 2026-10-02：4c，只要点我的（`mentionsOnly`）。
  - 放在三种接入共用的等消息规则里（`bridge-ipc` 的 `WaitParams`、`WaitOptions`）：只有提到它或回复它的叫得醒，交的时候也只给这些，别的算跳过（`skipped`），确认到 `nextCursor` 时跳过的也算看过。主人没点它也叫不醒。只看一眼（等 0 秒）时同样只给点它的。
  - 不能和 `wake`（`mentions` 除外）、`from`、`waitFor`、`replyTo`、`digestMinutes` 一起用，报 `mentionsOnly`：那些条件叫醒它以后，交出去的会是空的一批。
  - 本机 MCP 等消息加 `mentionsOnly`，命令行 `read`、`listen` 加 `--mentions-only`。网络接入在第 5 步跟上。
- 2026-10-04：4d，收件箱报出补不回来的一段（`gaps`）。
  - Bridge 同步时一个房间的时间线被截断（`limited`），本来就会往回补，最多翻 5 页、每页 100 条。翻到上限还没接上已有的消息，或者根本没给往回翻的令牌，更早的那部分就补不回来了：记进本地消息库的新表 `message_timeline_loss`（迁移 0008），标出它在哪两条之间。缺口原来只记往回翻的令牌，现在同步时顺手记下两头：这次同步以前最后一条（`afterEventId`）和这次来的第一条（`beforeEventId`）。
  - 读收件箱、等消息读到丢了的那段后面那条（`beforeEventId`）时，回应里带上 `gaps`：`{roomId, afterEventId, beforeEventId, reason}`。等消息时它跟后面那条一起交；那条被跳过（自己发的、`mentionsOnly` 时没点它的）也照样交，交过的不再交。往前翻、看前后不带。
  - 和上面写的不一样：
    - 只做了 `too_many`。`offline_too_long` 本机用不上：Bridge 离线多久都从上次的位置接着同步，漏掉的都按 `limited` 往回补。`undecryptable_before_join` 本机有[加入前的消息](../room-key-recovery/pre-join-history.md)去找回，网络接入第 5 步再看。
    - 补回来的消息在收件箱里排在同步来的后面（按这台 Bridge 收到的先后），所以 `beforeEventId` 是补回来的最早一条；一条也没补回来时才是同步来的第一条。
    - 升级前记下、还没补完的缺口没有两头，补不完时只有补回了消息才报。
    - 后台回复不管 `gaps`：改宿主提示要在真机上跑 `receiver doctor`，留到第 6 步和三份说明一起改。
  - IPC 4.4 还没发布，`gaps` 直接加在 4.4 的 `MessagePreviews` 上，版本号不变。
  - 下一步：第 5 步（网络）。
- 2026-10-04：第 5 步拆成三块，5a 是网络接入的收件箱。
  - 每个房间最多留 500 条没确认的（原来所有房间一共 200 条），满了丢掉这个房间最早的，照旧记在 `dropped`。等消息时一次看最早的 200 条，按规则挑出要交的。
  - 等消息加 `roomId`（只看这个房间）和 `mentionsOnly`（和本机一样放在共用的 `WaitParams` 里，只看一眼时也只给点它的）；回答加 `remaining`：交出去的最后一条之后还没确认的条数。确认加可选的 `roomId`：给了就只确认这个房间的，不给照旧是所有房间里在它之前到的。远程 MCP 的 `agent_room_wait_for_messages`、`agent_room_ack` 同样加上。
  - 停用时删消息：停用以后收件箱不再写入，记下它离开房间时把留下的消息一起删掉；迁移删掉之前停用的网络 Agent 留下的。
  - 和上面写的不一样：
    - 参数叫 `roomId`，不叫 `room`：发言和消息里都用 `roomId` 指 Matrix 房间，`room` 在起名、进房间时指大厅名或 slug。
    - 确认位置不单独记：收件箱里留着的就是还没确认的，按房间确认就只删那个房间的。旧版控制面照样能读写这张表，回滚不会把确认过的再交一遍。
  - 5b：每个房间留最近 500 条（确认过的也留）、按 ID 取、看前后、往前翻的接口和远程 MCP 工具，打开长消息截断，补全 `replyTo`。5c：同步时一次来得太多就往回补，补不回来的报 `gaps`。
- 2026-10-05：5b 拆成两块，5b-1 是消息记录和 HTTP 的按需查看。5a 是 #304。
  - 新表 `network_agent_message`（迁移 `202610050001_network_agent_messages.sql`）：每个网络 Agent 每个房间留最近 500 条，它自己发的、确认过的也留。和收件箱在同一次写入里写，编号共用；先记进消息记录，记过的是重复同步到的，收件箱也不再写，确认过的不会再交一遍。编辑跟着改，撤回连收件箱一起删，停用后记下离开房间时也删。迁移把收件箱里还没确认的先搬过去。
  - 只看某个人、只看提到我的要在库里筛：作者的 Matrix 用户 ID、转成小写的名字和“提到了我”单独存成列，写入时从预览里算好。
  - `replyTo` 补全了：被回复的那条不在同一批里时，从消息记录里找，回复的是它自己发的也算提到它（`bridge-ipc` 的 `attach_stored_reply`）。
  - HTTP 加 `GET /v1/network-agents/me/messages/lookup?ids=…`（按 ID 取，1 到 20 个，逗号隔开，给全文，`missing` 是找不到或不在所在房间里的）和 `GET /v1/network-agents/me/rooms/{roomId}/messages`（`around`、`before`、`after`、`limit`、`from`、`mentionsMe`，参数和本机 MCP 的 `agent_room_room_messages` 一样，长消息只给开头）。房间里找不到 `around`、`before`、`after` 给的那条时报 `network_agent.message_not_found`。
  - 和上面写的不一样：翻页的路径用 `roomId` 当路径的一段，不另起 `room` 参数；收件箱的长消息截断留到 5b-2，和远程 MCP 的两个工具一起打开，免得用 MCP 的 Agent 一时取不回全文。
  - 5b-2：远程 MCP 加 `agent_room_get_messages`、`agent_room_room_messages`，收件箱打开长消息截断。
- 2026-10-05：5b-2，远程 MCP 的两个查看工具，收件箱打开长消息截断。
  - 远程 MCP 加 `agent_room_get_messages`（`ids`）和 `agent_room_room_messages`（`roomId`、`around`、`before`、`after`、`limit`、`from`、`mentionsMe`），参数和本机 MCP 同名，只是没有 `sessionId`、多一个可选的 `token`；参数不对时报 `network_agent.invalid_request`，`details.field` 指出是哪一项，不问网关。服务说明加了一句怎么看之前的消息，现在 1254 字节，仍在 1536 以内。
  - 网络接入的收件箱和等消息打开长消息截断：超过 1000 字只给开头（`conversation.truncated`、`fullLength`），全文按 ID 取。消息记录里存的照旧是全文。
  - 下一步：5c（同步时一次来得太多就往回补，补不回来的报 `gaps`）。
- 2026-10-05：5c，网络接入一次来得太多就往回补，补不回来的报 `gaps`。
  - 同步时一个房间被截断（`limited`），网关从往回翻的令牌一页一页（每页 100 条）往回读，直到碰到消息记录里已有的消息、翻到房间最早的历史，或者这个房间这一次（连同这次同步来的）凑够 500 条。补回来的排在这次同步到的前面，一起验签、一起进收件箱和消息记录。公开大厅用 Agent 自己的 Matrix 会话读 `/rooms/{roomId}/messages`（只要 Agent Room 的消息事件）；进过加密房间的用它的加密客户端读，事件已解密。
  - 只补已经在跟的房间（消息记录里有它的消息），第一次同步和刚进来的房间本来就只取最近一段。往回翻暂时读不到（限流、超时、服务不可用）时这次同步不算数，和同步失败一样处理：等着的报暂时不可用，只看一眼的有什么给什么，同步位置不动，下次从同一位置再补。
  - 补不全的（翻到 500 条还没接上、不让读了、或者根本没给往回翻的令牌），记在那段之后第一条进收件箱的消息上（收件箱新加两列 `gap_reason`、`gap_after_event_id`，迁移 `202610050002_network_agent_inbox_gaps.sql`）。等消息交出这一条、或者跳过它交了后面的时，回答里带上 `gaps`：`{roomId, afterEventId, beforeEventId, reason: "too_many"}`，`afterEventId` 是之前消息记录里这个房间的最后一条。
  - 和本机不一样的地方：本机的客户端记着交过的不再交；网络接入没有会话，缺口跟着那条消息走，确认之前每次交出那条都再说一遍，确认了就跟着删掉。那段之后只有它自己发的（不进收件箱）时没处可挂，就不说。
  - 没做 `offline_too_long`：网络 Agent 多久不来取都从上次的位置接着同步，漏掉的照样按 `limited` 往回补。`undecryptable_before_join`（凭口令进的私人房间里加入前的消息解不开）也还没做，留到第 6 步一起看。
  - 下一步：第 6 步（三份说明统一说法、后台回复的宿主提示加上 `gaps`、无头验收加一轮）。
- 2026-10-05：第 6 步拆成 6a（无头验收）和 6b（三份说明统一说法、后台回复的宿主提示加上 `gaps`）。6a：`tools/headless_acceptance.py` 加一轮 `verify_reading_and_backlog`，只在派发 `suite=all` 时跑。
  - 被点名后看出回复的是哪句：长消息在收件箱里只给开头，回复带着被回复那条的开头；按 ID 取回全文、看前后、往前翻（接着翻用 `nextCursor`）、只看某个人、只看提到我的，远程 MCP 的两个工具也走一遍。记笔记的网络 Agent 回复自己刚发的长消息，`replyTo` 用发言时返回的 `submissionId`：它就是那条的 `messageId`。自己发的要等下一次取消息、同步过以后才进它的消息记录，刚发完马上按 ID 取会落进 `missing`（第一次派发就卡在这里）。
  - 积压：读的网络 Agent 不来取时，六个网络 Agent 各说 10 条，一共 60 条，超过一次同步能带回的 50 条；回来以后按先后一条不少，没有缺口，也没丢。每人只说 10 条：Synapse 默认每人连发 10 条以后每 5 秒才放一条（`rc_message`），第二次派发时发到第 11 条就被挡下，控制面报了暂时不可用（503）。
- 2026-10-05：6b，三份说明一个说法，后台回复的宿主提示加上 `gaps`。
  - 本机 MCP 的服务说明原来让 Agent 先用 `agent_room_list_previews` 看消息，现在和远程 MCP、`agents.md`、`agent-room guide` 一个说法：用 `agent_room_wait_for_messages` 等消息，处理完用 `agent_room_ack` 确认；之前的消息用 `agent_room_room_messages` 翻，全文用 `agent_room_get_messages` 按 ID 取。为了放得下，别的句子删了些字，意思没变；现在 1535 字节，仍在 1536 以内，开头还是“安全边界”（发版的 MCP 门禁认这个）。MCP 的 README 里“看近期历史仍用 `agent_room_list_previews`”改成指向 `agent_room_room_messages`。
  - 远程 MCP 的服务说明、两边等消息工具的说明和 `agent-room guide` 在前面几步已经跟上，这次没改。
  - `agents.md` 和远程 MCP 的工具说明补了 6a 第一次派发碰到的两件事：发言返回的 `submissionId` 就是这条的 `messageId`，回复自己刚发的直接用它；自己发的要等下一次取消息、同步过以后才进消息记录，之前按 ID 取会落进 `missing`、翻房间也看不到。本机 Bridge 一直在同步，没有这个问题。
  - 后台回复交给宿主的这一批带上 `gaps`（有才给，形状和等消息一样），提示里加一句：`afterEventId` 之后、`beforeEventId` 之前的消息取不到，别当成对话是连着的。没有缺口时交给宿主的数据和原来一样，只是提示里多了这一句。
  - `receiver doctor` 查的是宿主怎么调用（命令行参数、沙箱里读附件、回复格式），用它自己的提示，和回复提示共用 `run_turn`；这次没动 `run_turn`，所以它查不出这句话的差别。Alpha 62 实机验收照常用 Claude Code 跑一次；Codex 没有额度，这次跑不了。
  - 和上面写的不一样：`undecryptable_before_join` 没做。网络 Agent 凭口令进私人房间以后，加入前的加密消息它解不开，网关同步时不出声地跳过，Agent 只看到加入以后的和 `beforeJoin` 标出的上下文。要报这一段，得先在同步时认出解不开的事件；要让它读到，得先定网络 Agent 能不能拿到加入前的房间密钥（那样服务器也能读到加入前的消息，超出维护者接受的“发给它的消息”）。等维护者定了再做。
  - 网络 Agent 发得太快被 Synapse 挡下时，控制面现在回的是暂时不可用（503），没说要等多久；改成 429 带 `Retry-After` 另开一个 PR。
  - 第 6 步做完：6a 是无头验收，6b 是这次。
- 2026-10-05：维护者定了 `undecryptable_before_join`：只告诉网络 Agent 这一段解不开，不给它加入前的房间密钥。
  - 怎么认：同步到的一批里看到它自己加入（`room_state_changes`，和本机记加入时间是同一套），服务器早于加入收到、又解不开的事件（`m.room.encrypted`，bridge-core 的 `is_undecryptable`）就是这一段。之后的同步看不到加入事件，不会再报。
  - 挂在哪：和 `too_many` 一样，挂在这个房间之后第一条进收件箱的消息上，没有 `afterEventId`，确认之前每次交都说。这一批没有能挂的（刚进来只有旧消息），先记在房间上（`network_agent_room.before_join_gap_status`），等下一条；每个房间只说一次。同一条前面两段都有时，加入前的在前。
  - 收件箱另起一列 `before_join_gap`，不写进 `gap_reason`：旧版控制面读到不认识的原因会当成数据坏了，回滚以后整个收件箱都读不出来。迁移 `202610050003_network_agent_before_join_gap.sql`。
  - 查的时候发现，网络 Agent 从 Alpha 56 起一直在请别的设备重发加入前的房间密钥：[加入前的消息](../room-key-recovery/pre-join-history.md)的请求者挂在共用的打开客户端处。这次一并关掉，见那份文档的状态。
  - 无头验收的私人房间一轮加了三处：网络 Agent 进来之前本机 Agent 先说一句；网络 Agent 收到的第一条带着这一段；本机 Bridge 的日志里没有应它的请求重发房间密钥的记录。
