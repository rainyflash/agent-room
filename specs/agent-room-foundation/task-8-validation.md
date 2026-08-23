# 任务 8 Outbox 与 Matrix 投影验证记录

> 验证日期：2026-08-23
> 结论：通过
> 对应任务：[实施计划 8](./tasks.md#m1内部纵向切片)
> 核心实现提交：`29270ca`、`04ecd9e`、`92eb349`

## 1. 事务 Outbox 与可靠消费者

- `AgentRegistrationTransaction` 是 Agent 注册及 `agent.registered.v1` 事件的单一事务端口；PostgreSQL 适配器在一个 SQLx 事务内写 Agent、Owner 和 Outbox，任一约束失败都会整体回滚。
- Outbox 领取使用确定排序、行锁和 `SKIP LOCKED`，并写入有到期时间的 Worker 租约。消费者崩溃后无需人工解锁，其他实例可在租约到期后接管。
- `OutboxProcessor` 统一发布流程：成功确认、瞬时失败有界指数退避、永久失败直接死信、达到最大尝试次数后死信。下游发布端必须使用 Outbox Event ID 作为幂等键。
- 发布成功但数据库确认失败时，事件保留租约并在到期后重放；系统不伪装成全局事务，而是依靠稳定幂等键收敛未知提交状态。
- `OutboxBacklog` 分别暴露可领取、退避中、租约中、死信数量和最老待处理时间，不用一个无法诊断的总数掩盖队列状态。

## 2. Matrix 投影一致性

- 增量批次携带 `expected_sync_token` 和 `next_sync_token`。适配器先锁定消费者游标，只有期望令牌精确匹配才允许推进；Matrix 令牌被当作不透明值，不做伪造的大小比较。
- 每个 Matrix 事件写入 `(consumer_name, event_id, event_digest)` 回执。相同 ID/摘要是重复投递，相同 ID/不同摘要映射为 `CorruptData`，整个批次回滚。
- 回执、房间成员状态、加入人数、活动度和同步游标处于同一 PostgreSQL 事务。提交后的整批重放只验证回执，不重复增加活动度或改变领域副作用。
- 房间人数从当前 `join` 成员重新计数，不依赖脆弱的手工加减；活动度只对首次处理事件累加。
- 快照重建在事务级 Advisory Lock 下清空可丢弃投影和回执、重置人数/活动度并重放完整快照。重复重建得到相同结果。

## 3. 安全读取策略

- `ProjectionFreshnessPolicy` 只允许健康且未超过配置年龄的游标支撑安全读取。
- 游标缺失、消费者异常、投影过期或本机/服务端时钟倒退时，读取计划明确返回 `QueryMatrix*`，调用方必须回源 Matrix 权威接口；权威接口不可用时应拒绝高风险操作，不能相信陈旧投影。
- 成员投影仍是查询模型，Matrix 房间状态始终是成员关系与权限的事实源。

## 4. 真实 PostgreSQL 故障验收

`python tools/database.py test` 每次创建名称固定的隔离数据库，执行全部迁移和 7 个真实数据库测试，完成后强制断开并删除测试库。任务 8 新增场景全部通过：

| 验收项 | 结果 |
| --- | --- |
| Agent 与 Outbox 原子创建 | 两者同时存在 |
| Outbox 主键冲突 | Agent 与 Owner 完整回滚 |
| 消费者在有效租约期间竞争 | 第二个消费者领取不到事件 |
| 原消费者崩溃且租约到期 | 第二个消费者成功接管并确认 |
| 瞬时发布失败 | 事件进入未来重试时间 |
| 达到失败阈值 | 事件进入可观察死信，不再被领取 |
| 投影提交后调用方崩溃并整批重放 | 返回 `Replayed`，人数和活动度不变 |
| 重复 Matrix Event ID/相同摘要 | 安全跳过，游标可继续推进 |
| 重复 Matrix Event ID/不同摘要 | `CorruptData`，游标和投影均回滚 |
| 旧 `expected_sync_token` 回退 | `Conflict`，新游标不被覆盖 |
| 完整快照连续重建两次 | 成员、人数和活动度结果一致 |
| 消费者健康变为 `lagging` | 查询上下文同步暴露异常状态 |

## 5. 质量门禁

- `just check` 全部通过：Rust/TypeScript 格式、Clippy `-D warnings`、类型检查、构建、单元测试、协议生成一致性、密钥扫描和 GitHub Actions 固定版本检查均无失败。
- `just coverage` 合并普通 Workspace 测试和真实 PostgreSQL 测试后，Rust 行覆盖率为 77.56%，高于 60% 门禁；`application/outbox.rs`、`postgres-adapter/outbox.rs`、`postgres-adapter/projections.rs` 行覆盖率分别为 93.72%、91.10%、93.74%。
- TypeScript 语句、分支、函数和行覆盖率均为 100%。
- `cargo deny check` 的 advisories、bans、licenses 和 sources 全部通过；使用 npm 官方审计端点执行 `pnpm audit --audit-level high` 未发现已知漏洞。
- Windows MSVC 的本地化 `linker_messages` 仍是无害提示，不影响编译或测试结论。

## 6. 实现依据

- PostgreSQL 官方文档明确指出 `SKIP LOCKED` 适合多个消费者访问队列表的场景，同时不适合作为一般一致性查询：[PostgreSQL SELECT 锁定子句](https://www.postgresql.org/docs/current/sql-select.html#SQL-FOR-UPDATE-SHARE)。
- Matrix `/sync` 使用上一次响应的 `next_batch` 作为下一次 `since`，并要求客户端按 Event ID 去重跨 API 重复事件：[Matrix Client-Server API：Syncing](https://spec.matrix.org/latest/client-server-api/#syncing)。
- SQLx 事务必须显式提交或回滚，未结束事务在 Drop 时启动回滚；实现仍显式处理两条路径以保留稳定错误映射：[SQLx Transaction](https://docs.rs/sqlx/latest/sqlx/struct.Transaction.html)。
