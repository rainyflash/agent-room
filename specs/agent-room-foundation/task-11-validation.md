# 任务 11 Matrix 基础适配器验证记录

> 验证日期：2026-08-24
>
> 结论：通过
>
> 对应任务：[实施计划 11](./tasks.md#m1内部纵向切片)
>
> 核心实现提交：`52ccbca`、`0e99013`、`8042dfe`、`1088aff`、`70cd367`、`ce45637`

## 1. 架构边界

- 应用层定义 `MatrixClientFactory`、`MatrixGateway`、会话、房间、同步、事件、回执、回填、失败分类和恢复策略；用例只依赖这些端口，不依赖 Matrix SDK。
- `matrix-adapter` 是标准 Client-Server API 的出站适配器，负责 SDK 类型转换、协议错误翻译和有界网络调用。它不包含 Agent、房间权限或消息策略等产品业务规则。
- 领域层没有引入 Matrix、HTTP、Synapse 或数据库依赖。Matrix 标识在进入适配器前已经过应用边界校验。
- 适配器只通过公开 Matrix Client-Server API 工作，没有读取或写入 Synapse 内部数据库、内部缓存或管理端点。

## 2. 标准协议能力

| 能力 | 实现与验收 |
| --- | --- |
| 登录 | 使用标准密码登录请求，返回用户 ID、设备 ID 和脱敏会话令牌 |
| 恢复会话 | 从调用方提供的 Matrix 会话恢复同一用户和设备，不重新登录 |
| 创建房间 | 支持公开/私人可见性、标准房间预设、名称、主题、直接会话标志和初始邀请 |
| 成员生命周期 | 邀请、加入和离开均通过标准房间 API，权限失败映射为稳定应用错误 |
| 同步 | 以 `next_batch` 作为后续 `since` 游标，覆盖 joined、invited、left 和 knocked 房间 |
| 事件发送 | 使用调用方生成的稳定事务 ID，并返回事务 ID 到事件 ID 的确定映射 |
| 回执 | 支持公开阅读、私有阅读和完全阅读标记 |
| 历史回填 | 使用受限时间线的 `prev_batch` 调用标准 room messages API 向后分页 |

所有请求关闭 SDK 自动重试。调用方必须经过应用层 `MatrixRetryPolicy`，因此重试次数、退避上限和事务 ID 都由 Agent Room 控制，而不是交给不可观察的 SDK 默认行为。

## 3. 幂等、同步与恢复语义

- 每个发送请求在进入适配器前就必须携带稳定事务 ID。相同 Matrix 设备、相同端点和相同事务 ID 的重复提交由 Homeserver 识别为同一请求；真实 Synapse 验证两次发送返回同一事件 ID。
- 同步映射保留发送设备在事件 `unsigned.transaction_id` 中收到的事务 ID。调用方可把远端回声与本地待提交记录对账，并拒绝一个事务 ID 映射到多个事件 ID。
- 发送在超时后被分类为 `UnknownCommit`，恢复动作只能是 `ReconcileSubmission`，不能盲目创建新事务 ID 重发。创建房间等非幂等操作超时同样不会自动重试。
- 同步返回的 `next_batch` 是唯一增量游标；受限时间线同时暴露 `prev_batch` 供历史回填。真实测试刻意把时间线上限设为 3，确认 6 个发送事件触发受限结果并能取回更早消息。
- 失效同步游标触发 `ResetSyncCursor`，失效会话触发 `Reauthenticate`，两者不会混成普通网络重试。
- 限流、超时和依赖不可用采用有界指数退避；服务端 `retry_after_ms` 仍受本地最大延迟约束。真实 Synapse 返回 429 时，重试保持原事务 ID。

## 4. 输入、安全与失败边界

- Homeserver 生产地址只允许 HTTPS；明文 HTTP 仅允许严格回环地址。URL 不得包含用户名、密码、查询参数或片段。
- 单次网络请求有 1 毫秒到 2 分钟的硬边界；同步长轮询参数最大 60 秒，SDK 自动重试关闭，防止一次调用暗中膨胀为不可控的分钟级阻塞。
- 同步时间线默认最多 50 条、配置上限 1000 条；应用事件内容最多 64 KiB，适配器在解析原始事件前设置 128 KiB 上限。
- 访问令牌、刷新令牌和同步游标的调试输出均脱敏。错误只暴露稳定分类，不把响应正文、凭据或底层网络细节带入应用层。
- Matrix 标准错误映射为 `Unauthenticated`、`AuthenticationRejected`、`Forbidden`、`NotFound`、`Conflict`、`RateLimited`、`Timeout`、`DependencyUnavailable`、`InvalidResponse`、`UnknownCommit`、`StaleSyncToken` 和 `UnsupportedVersion`。

## 5. 真实 Synapse 与故障验收

测试对象为项目固定的 Synapse 1.159.0。测试创建独立临时房间并在结束时让双方离开，不复用产品大厅或访问 Synapse 数据库。

| 验收项 | 结果 |
| --- | --- |
| 开发者与 Agent 标准密码登录 | 成功，用户和设备会话可恢复 |
| 未受邀 Agent 加入私人房间 | 返回 `Forbidden` |
| 邀请后同步与加入 | 依次收到 invited 和 joined 更新 |
| 相同事务 ID 重复发送 | 两次响应返回相同事件 ID |
| 发送方远端回声对账 | 事务 ID 精确映射到事件 ID |
| 时间线截断与历史回填 | 返回 `limited`、`prev_batch`，并取回更早事件 |
| 阅读回执 | 标准回执请求成功 |
| 离开后的增量同步 | 收到 left 房间更新 |
| Homeserver 429 | 遵守服务端延迟和本地上限，保持原事务 ID |
| TCP 接受后立即断开 | 映射为 `DependencyUnavailable` |
| TCP 接受后持续无响应 | 在适配器预算内映射为 `Timeout` |

## 6. 质量与供应链门禁

- `just check` 全部通过：Rust/TypeScript 格式、Clippy `-D warnings`、类型检查、构建、测试、协议一致性、Secret 扫描和 GitHub Actions 固定版本检查均无失败。
- `just coverage` 从干净覆盖率状态运行普通测试、真实 PostgreSQL 测试和真实 Synapse 测试。Rust 总行覆盖率为 78.74%，高于 60% 门禁；Matrix SDK 适配、事件映射和应用 Matrix 模型行覆盖率分别为 87.95%、88.57% 和 86.24%。TypeScript 四项覆盖率均为 100%。
- CI 在创建 Matrix 测试主体后运行同一套合并覆盖率流程，不再把真实 Synapse 验收排除在覆盖率度量之外。
- `cargo deny check` 的 advisories、bans、licenses 和 sources 通过；Node 官方审计端点未发现已知漏洞。
- Matrix SDK 0.18.0 传递依赖两个信息级“无人维护”公告：`RUSTSEC-2026-0173` 和 `RUSTSEC-2026-0247`。项目只按公告编号设置带移除条件的临时例外，没有关闭 advisories。SDK 主线已移除前者的依赖链，但尚未发布稳定版；后者暂无修复版本，必须继续跟踪。
- `xxhash-rust` 使用 OSI 批准的 Boost Software License 1.0。许可证白名单明确使用 SPDX 标识 `BSL-1.0`，并在配置中注明它不是 Business Source License。

## 7. 明确不属于本任务的内容

- 当前使用内存 Store，只验证基础 Client-Server 能力。Bridge 的持久 Matrix Store、单实例锁和自动重连属于任务 13。
- 私信和私人房间的端到端加密、设备验证、密钥恢复与多设备攻击验收属于任务 27。任务 11 没有伪称基础适配器已经满足 E2EE 发布门禁。
- Agent 独立 Matrix User、所有权和实例绑定属于任务 12；本任务仅提供可组合的协议端口。

## 8. 实现依据

- Matrix Client-Server API v1.19 定义登录、同步、房间、事件发送、事务标识、回执与历史分页：[Matrix Client-Server API](https://spec.matrix.org/latest/client-server-api/)。
- Matrix 事务 ID 用于让 Homeserver 区分新请求和重传，并把同设备、同端点的请求变成幂等操作：[Matrix Transaction identifiers](https://spec.matrix.org/latest/client-server-api/#transaction-identifiers)。
- `prev_batch` 可用于 room messages API 回填受限同步之前的事件：[Matrix Syncing](https://spec.matrix.org/latest/client-server-api/#syncing)。
- 适配器锁定当前稳定版 [matrix-rust-sdk 0.18.0](https://github.com/matrix-org/matrix-rust-sdk/releases/tag/matrix-sdk-0.18.0)，不追踪未经发布的主线快照。
- 两个传递依赖公告均为信息级无人维护通知，而非已知可利用漏洞：[RUSTSEC-2026-0173](https://rustsec.org/advisories/RUSTSEC-2026-0173.html)、[RUSTSEC-2026-0247](https://rustsec.org/advisories/RUSTSEC-2026-0247.html)。
- `BSL-1.0` 是 OSI 批准的 Boost Software License 1.0：[Open Source Initiative](https://opensource.org/license/BSL-1.0)。
