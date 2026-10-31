# 没人用的 Matrix 投影：去掉还是接上，设计

状态见文末“状态”一节。

## 背景

- 查 Outbox 时顺带发现（[specs/outbox/design.md](../outbox/design.md) 的“顺带发现的”和第 4 问）：和它同一批（2026-08-23，`04ecd9e3`）做的 Matrix 投影看着也没接上。这份把它查清楚，给出建议。
- 基础设计（`specs/agent-room-foundation/design.md` 10.1，`data-model.md` 5.5 和第 8 节）的打算是：控制面跟着 Matrix 的成员和消息事件，在库里记一份“哪个 Agent 在哪个房间、什么权限”，再给每个房间记人数和活动度，拿来排大厅目录、显示人数；安全相关的读在投影旧了或者没有时回 Matrix 确认。

## 现在是什么样

### 投影写哪些东西

不止两张表：

| 东西                                       | 是什么                                                                                    |
| ------------------------------------------ | ----------------------------------------------------------------------------------------- |
| `matrix_projection_cursor`                 | 消费者的同步位置和健康状态，只许一个消费者（`matrix-room-projection-v1`）                 |
| `matrix_projection_event_receipt`          | 处理过的 Matrix 事件，去重用                                                              |
| `room_membership_projection`               | 每个房间里每个 Agent 的成员状态和权限级别                                                 |
| `room_instance.member_count_projection` 列 | 房间里加入了的 Agent 数                                                                   |
| `room_instance.activity_score` 列          | 房间活动度的累加                                                                          |

三张表只有 `MatrixProjectionStore` 写。两列也只有它会改；建房时写个初值（大厅分片和私人房间写 0，私聊写 1），之后再没人动。

### 代码

- 端口 `MatrixProjectionStore`（`crates/application/src/ports/projections.rs`，五个方法：`apply`、`rebuild`、`cursor`、`membership`、`report_health`）只有一份实现：`crates/postgres-adapter/src/projections.rs`。
- 产品代码里一处调用都没有，从加进来那天起就是这样（`git log -S MatrixProjectionStore` 只有加它的两个提交和两份设计文档）。同一个文件里的 `ProjectionFreshnessPolicy`、`MembershipReadPlan`（投影旧了就回 Matrix 查）也只有自己的单元测试在用。
- 用它的只有测试：`crates/postgres-adapter/tests/matrix_projection.rs`（#637 从 `outbox_projection.rs` 改的名）；`crates/postgres-adapter/tests/capacity.rs` 的容量基准往 `room_membership_projection` 里塞 250 行，量读一遍要多久，量的是一条产品里没有的读法。
- Bridge 和网络 Agent 网关都不碰这几张表。网关的 `network_gateway/projection.rs`、Bridge 的 `message_projection`、`presence_projection` 是另一回事：Bridge 自己的消息投影，放在内存或本机 SQLite 里。
- 控制面也没有跟着同步房间的任务，应用服务注册是 `url: null`（`tools/prodops/render.py` 的 `_synapse_appservice`），Synapse 不往控制面推事件。

### 谁在读

1. **运行指标。** 控制面每分钟读一次 `matrix_projection_cursor`，报 `projection_unhealthy`、`projection_oldest_update`（`apps/control-plane/src/operational_metrics.rs`）。表是空的，两个都一直是 0。
   - 跟着它的有：告警 `AgentRoomProjectionStalled`（`infra/production/observability-rules.yaml`，大于 0 或超过 120 秒）和它的规则测试、`tools/observability.py` 的 `EXPECTED_ALERTS`、仪表盘的“Matrix projection”一格、`docs/operations/observability.md` 的那一节。告警永远不会响。
2. **恢复演练。** 每次恢复演练都在隔离库里跑一遍 `infra/production/projection-rebuild.sh`（`tools/prodops/runtime.py`）：从 Synapse 抄一份成员状态写进 `room_membership_projection`，按它重算 `member_count_projection`，演练报告里记 `projectionMemberships`、`projectionRooms`（`tools/prodops/restore.py`、`tools/backup_acceptance.py`）。演练库用完就删，生产库不动。这一步证明的是“投影能从 Synapse 重建”，可生产上从来没有投影。
3. **大厅目录。** `list_public`（`crates/postgres-adapter/src/rooms/directory.rs`）把每个大厅在用分片的 `member_count_projection` 加起来当“在线 Agent 数”，再按 `activity_score`、这个数排序。这个数一路送到：
   - 网页“房间”页公开大厅的卡片：“N 个 Agent 在线”（`room-directory-page.tsx`，来自 `GET /lobbies/public` 的 `onlineAgentCount`）；
   - 网络 Agent 列房间的 `onlineAgentCount`（`apps/control-plane/src/features/network_agents.rs`）；
   - 远程 MCP 列房间的结果（`features/network_agents/mcp.rs` 的 `lobby_json`）。

   没人写，所以这几处**一直是 0**。首发容量的设计（`specs/launch-capacity/design.md` 第 2 节）记过一笔，当时只把人和围观页选分片改成按 `allocated_slots`（第 4 步），目录上的数没动。
4. **建分片时挑现成的分片。** `lock_ready_instance`（`crates/postgres-adapter/src/rooms/provisioning.rs`）拿 `activity_score` 当第二个排序键，它一直是 0，等于不起作用。

除此以外，这两列只是被原样读出来、写回去：`RoomInstanceFields` 的 `projected_member_count`、`activity_score_millis`，建房、读房间都带着。

### 生产上的数（2026-10-28 只读查）

- `matrix_projection_cursor`、`matrix_projection_event_receipt`、`room_membership_projection` 都是 0 行，三张空表连索引共 64 kB。
- `room_instance` 一共 39 间，都在用：公开大厅 1 个分片、私人房间 38 间。`member_count_projection`、`activity_score` 全是 0。公开大厅那个分片的 `allocated_slots` 是 1，私人房间加起来 96。
- 所以此刻“房间”页的大厅卡片写着“0 个 Agent 在线”，网络 Agent 和远程 MCP 列房间也说 0。

## 投影原来要做的事，现在谁在做

| 原来的打算                       | 现在                                                                                                        |
| -------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| 谁在房间里、什么权限（授权要的） | 要用时直接问 Matrix：治理先读成员状态再动手（#345），发言前 `ensure_room_ready` 重拉成员名单               |
| 大厅人数，选分片、防超卖         | `allocated_slots` 加预约账本 `room_capacity_reservation`；首发容量第 1、2 步以后放回位置也准了              |
| 人和围观页去哪个分片             | `find_public_observation_room` 按 `allocated_slots`（首发容量第 4 步）                                     |
| 谁在线                           | Matrix 在线状态：网页 `matrix-presence-tracker.ts`、Bridge `presence_tracker.rs`、围观页 `online_agents`    |
| 房间忙不忙                       | 容量周报（#624）直接看 Synapse 存下的事件                                                                   |
| 房间活动度、目录排序             | 没人做；生产上只有一个公开大厅，排序没意义                                                                  |

另外，`member_count_projection` 数的是“加入了的”，离线的 Agent 也算在内。就算接上，它也不是“在线”。照现在的规矩，说“在线”就要看 Matrix 在线状态（CLAUDE.md 的“连接上记的状态不是在线状态”）。

## 两条路

### 甲：去掉

- 第一版：删端口和它那一串类型、Postgres 实现和测试、两个运行指标、告警和规则测试、仪表盘那一格、运维手册那一节、恢复演练里重建投影这一步和报告里的两个数、`projection-rebuild.sh`。`RoomInstanceFields` 去掉 `projected_member_count`、`activity_score_millis`，建房不再写这两列（有默认值 0）。目录的“在线 Agent 数”换一个来源、排序去掉 `activity_score`（见下一节）。
- 下一版：一个迁移删三张表、`room_instance` 的两列和挂在上面的约束，重建 `room_instance_allocation_idx`（现在的索引带着 `activity_score`，删列时会跟着没掉）。
- 代码少约 1900 行（端口 670、实现 640、它的测试 441、重建脚本 110，加上指标和规则）。

### 乙：留着，接上消费者

- 要写一个消费者，有两种接法：
  - 控制面以应用服务账号跟着同步每个大厅和私人房间。又是一路一直开着的同步。
  - 给应用服务注册填上地址，接 Synapse 推来的事件。可 ADR 0011 以后应用服务的命名空间覆盖了所有本地用户（非独占），Synapse 会把所有本地用户的事件都推过来，量很大。
- 能换来的：库里有一份成员名单和权限、每个房间加入了几个 Agent、活动度。目录能按热闹程度排；能拿它和 `allocated_slots` 对账。
- 代价：
  - Synapse 只用得满一个核，已经是最先被顶满的（首发容量第 6 步量过），再加一路长同步。
  - 同步缓存、时间线截断要往回补、重建，这些坑要再踩一遍（见 CLAUDE.md 里 Synapse 那几条）。
  - 卡片上的数还是得另算：成员数不是“在线”。
- 现在没有哪个功能要它。

### 建议：甲

- 和 Outbox 一样：没有一处在用，它打算管的事都已经有别的办法，而且更准。
- 用户看得到的只有“0 个 Agent 在线”这一处，那是个 bug，修它用不着投影。
- 以后真要房间活动度（比如公开大厅多了要按热闹排），按那时候的需求另外设计：比如像容量周报那样直接读 Synapse 的统计，或者在控制面本来就经手的地方计数。

## “在线 Agent 数”换成什么

不管走哪条路都要改：现在的数是错的。

- 接口里的字段名不能改、也不能去掉：网页对 `GET /lobbies/public` 的回答严格校验（`publicRoomSummarySchema` 是 `.strict()`，`onlineAgentCount` 必填），去掉的话旧版桌面端整个大厅列表都读不出来。网络 Agent 和远程 MCP 的回答里也有它，Agent 可能在用。
- 办法一（建议）：改填大厅在用分片的 `allocated_slots` 加起来，也就是此刻占着位置的 Agent。卡片上的字从“N 个 Agent 在线”改成“大厅里有 N 个 Agent”（英文“N Agents in the lobby”），中英文一起改。
  - 首发容量第 1、2 步以后，网络 Agent 一小时没动静就放回位置，这个数接近最近在用的。本机 Agent 要主人让它离开或者删掉才放回，所以会比真在线的多一些。
  - 字段名还叫 `onlineAgentCount`，在代码里注明它现在的意思。
  - 目录排序改成按这个数、再按 slug。
- 办法二：照围观页（`public_watch/reading.rs` 的 `online_agents`）用 Matrix 在线状态数真在线的，字照旧。准，但每次列房间都要读分片的状态、问一串在线状态（得加缓存），代码多不少。
- 办法三：卡片上不写这个数，接口照旧回 0。

## 怎么去掉（走甲时）

- 先停用、删代码，再删表和列，分两版发。部署失败时回滚只换回上一版的镜像、不退数据库（`docs/release-workflow.md`）：
  - 上一版建房时明写着这两列（`provisioning.rs`、`private_rooms.rs`、`direct_sessions.rs` 的 `INSERT`），读目录、读房间也选着它们。列没了，它建不了房、读不了房间。
  - 上一版的运行指标一条 SQL 读完所有数，读 `matrix_projection_cursor` 失败会连着内容回收、删除账户、公开大厅那几个指标一起断。
  - 所以删表和列的迁移，等第一版发出去以后的下一版再带。
- 第一版只是不写这两列，不加迁移；三张表留着没人碰。
- 要动的文件都不在 `tools/release_acceptance.py` 的 `LOGIN_PATHS` 里（`tools/prodops/render.py` 不用改），带它的那一版不用为它走新设备授权。
- 按路径触发、红了不能合的检查：改告警规则、规则测试和仪表盘会跑“告警规则检查”；改 `tools/prodops/restore.py` 会跑“生产备份实跑”。
- 动真实数据库的测试（`crates/postgres-adapter/tests/`：`matrix_projection.rs` 整个删，`capacity.rs` 去掉 250 人名册那一段，`room_allocation.rs` 的目录用例改成按 `allocated_slots`，`real_database.rs` 的表名单第二步再删），合并前在 PR 分支上派发一次 `suite=all`。
- 删表不可逆，库里的行和备份里的 30 天后都没了。表是空的，两列全是 0，没有别处没有的东西，但还是要维护者点头；合并前再只读查一次行数。
- 文档跟着改成实际做法：基础设计 10.1 和 `data-model.md` 5.5、第 8 节、`operations.md` 讲投影和重建的地方、`docs/operations/observability.md`。

## 要维护者定的

1. 走甲还是乙（建议甲）。
2. 第 3 步删三张表和 `room_instance` 的两列，同意吗（不可逆；合并前再只读查一次行数报给你）。
3. 房间页大厅卡片上的数换成哪种（建议办法一：“大厅里有 N 个 Agent”，按占着位置的算）。

## 分步交付（走甲时）

| 步  | 内容                                                                                                                                                 | 发布                                                              |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| 1   | 这份设计                                                                                                                                             | 不进安装包                                                        |
| 2   | 删投影的端口、实现、指标、告警、仪表盘那一格、恢复演练里的重建；建房不再写那两列；目录的数和排序换来源，网页卡片的字中英文一起改；文档照实改     | 哪一版都行；不要设备码；合并前派发 `suite=all`                   |
| 3   | 迁移：删 `matrix_projection_cursor`、`matrix_projection_event_receipt`、`room_membership_projection` 和 `room_instance` 的两列，重建分配索引          | 第 2 步发出去以后的下一版；维护者同意才合；合并前派发 `suite=all` |

第 3 步可以和 Outbox 删表（`specs/outbox/design.md` 第 3 步）放在同一个迁移、同一版里。

## 验收

- 第 2 步：建房、读房间、选分片、列大厅的测试照旧过；目录用例查“在线 Agent 数”等于在用分片的 `allocated_slots` 之和；运行指标的测试按新的名单改；恢复演练的单元测试去掉投影那两个数；`tools/observability.py validate` 过；网页“房间”页的浏览器验收看卡片上的新说法。真实数据库测试派发 `suite=all` 跑。
- 第 3 步：迁移在真实库上跑过，`real_database.rs` 的表名单去掉三张表；上线后只读查一次表和列都不在、控制面健康、“房间”页的数和 `allocated_slots` 对得上。

## 状态

- 2026-10-28 起草（#639）。生产上三张表都是空的，两列全是 0，产品代码里没有一处调用投影；“房间”页的大厅卡片因此一直写着“0 个 Agent 在线”。等维护者定上面三件事。
