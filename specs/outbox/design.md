# 没人读的 Outbox：去掉还是接上，设计

状态见文末“状态”一节。

## 背景

- 2026-10-28 生产上打开监控以后，Outbox 积压的告警一直响（agent-room-ops#36）。查下来控制面只往 `agent_room.outbox_event` 里写，从来没有谁去读：`OutboxProcessor`（`crates/application/src/outbox.rs`）只在测试里用，测试以外也没有 `OutboxPublisher` 的实现。#635 去掉了这两条告警，仪表盘照旧显示，留了一句“要不要接上发布器，还是把 outbox 整个去掉，另开任务定”。这份就是那个任务。
- 基础设计（`specs/agent-room-foundation/design.md` 10.1）写的是“跨 PostgreSQL 与 Matrix 的动作采用 Outbox + 幂等消费者”。实际做下来不是这样：Matrix 那一步都在请求里直接做，要后台接着做的事各自有一张任务表。

## 现在是什么样

### 谁在写

| 事件                          | 什么时候写                                                       | 载荷                                |
| ----------------------------- | ---------------------------------------------------------------- | ----------------------------------- |
| `agent.registered.v1`         | 建 Agent（`postgres-adapter` 的 `agents.rs`、`agent_creation.rs`） | `owner_id`、`matrix_user_id`        |
| `agent.instance.registered.v1` | 登记 Agent 连接（`agent_instances.rs`）                           | `agent_id`、`device_id`             |
| `agent.instance.revoked.v1`   | 断开一个连接（`agent_instances.rs`）                              | `principal_id`                      |
| `agent.retired.v1`            | 删除 Agent（`agent_retirement.rs`）                               | `principal_id`                      |
| `agent.membership.changed.v1` | 改 Agent 的主人（`agent_memberships.rs`）                         | `principal_id`、`role`              |
| `device.revoked.v1`           | 撤销电脑（`devices.rs`）                                          | `deviceId`、`principalId`、`reason` |
| `device.compromised.v1`       | 刷新令牌被重复使用（`devices.rs`）                                | 同上                                |

都和业务写入在同一个事务里，写一行就完。

### 谁在读

- 没有。除了测试，只有控制面每分钟数一次待发布、死信和最旧的有多久，报成运行指标（`apps/control-plane/src/operational_metrics.rs` 的 `outbox_pending`、`outbox_dead_lettered`、`outbox_oldest_pending`），仪表盘上一格“Outbox”。

### 生产上的数（2026-10-28 只读查）

- 一共 413 行，248 kB：`agent.registered.v1` 194 行、`agent.instance.registered.v1` 193 行（都从 9 月 19 日起），`device.revoked.v1` 26 行（10 月 18 日一键撤销那批起）。别的四种一次也没写过。
- 一行也没发布、没被领取、没进死信。
- 撤销了的 Agent 连接 30 个，Matrix 设备都注销了（`matrix_device_revoked_at` 都有值）。
- 还没有人删过账户（`account_deletion_job` 是空的）。

## 每种事件该跟着做的事，现在谁在做

| 事件                   | 要跟着做的                        | 现在怎么做                                                                       | 没做成怎么办                                                     |
| ---------------------- | --------------------------------- | -------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| 建 Agent               | 在 Matrix 上开 Agent 的账户       | 写库之前先 `ensure_user`（幂等）                                                 | 请求失败，同一个请求 ID 再来一次（`agent_creation_request` 记着） |
| 登记连接               | 给这个连接一台 Matrix 设备和会话  | 写库之后马上 `rotate_device_session`                                             | 请求失败，Bridge 再登记一次、拿回原记录                          |
| 断开连接、撤销电脑     | 注销 Agent 的 Matrix 设备         | 请求里直接注销，记下 `matrix_device_revoked_at`                                  | 回 202，要人再点一次撤销；没有后台重试（见“顺带发现的”）         |
| 撤销电脑               | 它的 Agent 离开房间               | 撤销事务里记进 `revoked_agent_departure`，请求里做一遍，后台每 30 秒接着做（#546） | 退避重试                                                         |
| 删除 Agent             | 先离开它在的所有房间              | 接口里先 `leave_all`（#449），离开不成就不删                                     | 再点一次                                                         |
| 改主人                 | 没有：主人只记在控制面            | —                                                                                | —                                                                |
| 刷新令牌被重复使用     | 撤掉这一组令牌                    | 同一个事务里                                                                     | —                                                                |

所以 Outbox 里的事件没有一件事在等着它做。项目里要“后台接着做”的地方，都是一张专门的任务表加一个定时任务：删除账户（`account_deletion_job`）、撤销后离开房间（`revoked_agent_departure`）、开大厅分片（建房任务）、治理动作到期解除、网络 Agent 清理、正文对象清理。每张表只记这件事要的东西，有自己的租约和退避，出了错看得出是哪件事。

## 两条路

### 甲：去掉

- 停写；删掉 `OutboxProcessor`、Outbox 的端口和 Postgres 实现、事件 ID（`OutboxEventId`、`IdentifierFactory::outbox_event_id`）、三个运行指标和仪表盘那一格；基础设计里的说法照实改；最后加一个迁移删表。
- 建 Agent、登记连接、撤销时每次少写一行；代码连测试少约 1700 行。

### 乙：留着，接上消费者

- 要接就得说清楚消费者做什么。照上一节，七种事件里没有一件还没人做。能想到的只有：
  1. 撤销以后 Matrix 设备没注销成的，后台再试。这件事该做（见“顺带发现的”），但用不着 Outbox：`agent_instance` 上记着 `revoked_at` 和 `matrix_device_revoked_at`，迁移 `202608250002` 还专门建了找这些行的部分索引（`agent_instance_pending_matrix_revocation_idx`）。照 `revoked_agent_departure` 那样直接扫这张表最简单；从 Outbox 读的话，读到事件还得回这张表查“还有哪些没注销”，做的是同一件事。
  2. 新功能：刷新令牌被重复使用时给账户发一封信、撤销电脑时通知主人、给外部系统发 webhook。没人要过，现在也没有别的服务要听这些事件。
- 只接一个“读到就标成已发布”的消费者，等于让指标好看，什么也没做。不该这样做。
- 留着还要补：发布过的行定期删（不然一直长）；以后载荷里加了别的东西，删除账户时要一起清。

### 建议：甲

- 没有要它做的事；真要后台补做的那一件，从业务表直接做更简单，也更好查。
- 以后真有别的服务要听控制面的事件，到时候按要发的事件重新设计（发什么、给谁、留多久），比现在留着一套没人用、说法和实际又对不上的强。

## 删除账户和隐私说明

- 载荷里只有 ID：账户、电脑、Agent 的 UUID。Agent 的 Matrix ID 是 `@_agent_<Agent ID>:<服务器>`，由 Agent ID 推出来（`MatrixAgentLocalpart::from_agent_id`）。没有名字、邮箱、消息。
- 删除账户（`crates/postgres-adapter/src/accounts.rs` 的 `finalize_local`）留下匿名化的账户记录、换成“Deleted agent”的 Agent、撤销了的电脑，ID 都还在。Outbox 里的行没有比这些多出什么，隐私说明页说的“一条不含名字和邮箱的删除记录”（`privacy.delete.kept`）没有被打破。所以眼下不用为删除账户另清 Outbox；生产上也还没删过账户，没有要补清的。
- 走甲：删表以后这些行就没了，备份里的照“备份最多留 30 天”到期。隐私说明页没提过它，不用改。
- 走乙：删除账户的任务要不要清它，看载荷里放什么。只放 ID 就不用；放了名字或内容，就要在 `finalize_local` 里一起删，隐私说明页和 `specs/privacy-page/design.md` 在同一个 PR 里改。发布过的行也要按期删。

## 怎么去掉（走甲时）

- 先停写、删代码，再删表，分两版发。部署失败时回滚只换回上一版的镜像、不退数据库（`docs/release-workflow.md`）；上一版建 Agent、登记连接、撤销电脑都在同一个事务里写这张表，表没了这三件事都会失败。所以删表的迁移等停写的那一版发出去以后，下一版再带。
- 停写那一步要改 `crates/application/src/devices.rs` 和 `crates/postgres-adapter/src/devices.rs`（`DeviceSecurityEvent` 只剩时间），两个都在 `tools/release_acceptance.py` 的 `LOGIN_PATHS` 里：带它的那一版实机验收要走一次新设备授权，要维护者批准设备码。最好跟别的也改登录代码的 PR 放在同一版。
- 删表不可逆：库里的行没了，备份里的 30 天后也没了。这些行没有别处没有的东西（见上一节），但还是要维护者点头；合并前再只读查一次行数。
- 两步都动真实数据库的测试（`crates/postgres-adapter/tests/`），合并前在 PR 分支上派发一次 `suite=all`。
- 基础设计、数据模型、协议、运维文档和 `docs/architecture.md` 里讲 Outbox 的地方改成实际做法：跨系统的那一步在请求里直接做；要后台补做的，各自一张任务表加定时任务（租约、退避）。`specs/error-triage/design.md` 里列的“Outbox 死信”一起去掉（汇总的代码本来就没查它）。

## 顺带发现的

- 撤销以后 Matrix 设备没注销成时，没有后台重试（`specs/computer-revocation/design.md` 的“不做”里记过）。撤销那一刻 Synapse 不通，Agent 的 Matrix 设备就一直登着，要人再点一次撤销；桌面端换账号、移出这台电脑的那条路上，这台电脑的凭据已经删了，没人能再点。生产上现在一个也没有。建议另做一小步：照 `agent_room_departures.rs` 的样子，后台每 30 秒找 `revoked_at` 有值、`matrix_device_revoked_at` 为空的连接再注销一次，用上已有的部分索引。它和去掉 Outbox 互不依赖。
- 和 Outbox 同一批做的 Matrix 投影（`matrix_projection_cursor`、`matrix_projection_event_receipt`，`crates/postgres-adapter/src/projections.rs`）看着也没接上：`MatrixProjectionStore` 只有 Postgres 那一份实现，没有用例用它，控制面里只有运行指标读这两张表（表是空的，所以“投影停滞”的告警不会响）。没细查，要不要一起清另说。

## 要维护者定的

1. 走甲还是乙（建议甲）。
2. 第 3 步删表，同意吗（不可逆；合并前再只读查一次行数报给你）。
3. 撤销以后 Matrix 设备注销的后台重试，要不要做（建议做，另开一步）。
4. Matrix 投影那两张表要不要另查（建议另开任务）。

## 分步交付（走甲时）

| 步  | 内容                                                                                                                         | 发布                                                       |
| --- | ---------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| 1   | 这份设计                                                                                                                     | 不进安装包                                                 |
| 2   | 停写 Outbox；删 `OutboxProcessor`、端口、Postgres 实现、事件 ID、三个运行指标和仪表盘那一格；改各处文档里的说法             | 哪一版都行；要设备码；合并前派发 `suite=all`              |
| 3   | 迁移：`DROP TABLE agent_room.outbox_event`                                                                                   | 第 2 步发出去以后的下一版；维护者同意才合；合并前派发 `suite=all` |

第 2 步等 #635 合并以后再做：它也改仪表盘和 `docs/operations/observability.md`。

## 验收

- 第 2 步：建 Agent、登记连接、断开连接、撤销电脑、删除 Agent、改主人、刷新令牌被重复使用的测试照旧过，去掉其中“写了一行 Outbox”的断言；运行指标的测试按新的名单改；真实数据库测试派发 `suite=all` 跑。
- 第 3 步：迁移在真实库上跑过，`crates/postgres-adapter/tests/real_database.rs` 的表名单去掉它；上线后只读查一次表已经不在、控制面健康。

## 状态

- 2026-10-28 起草（#636）。等维护者定“要维护者定的”四条。
