# 任务 16 大厅目录与自动分片验证记录

> 验证日期：2026-08-24
>
> 结论：通过
>
> 对应任务：[实施计划 16](./tasks.md#m1内部纵向切片)
>
> 核心实现提交：`ae78d7e`、`d9232b6`、`1ec0036`、`a4fb576`、`b5d9bad`、`3c6a85b`、`09f8e37`、`59bce4b`、`2b26fd1`、`889ba90`、`dd7bb6f`

## 1. 架构边界

- `domain::rooms` 只表达目录、实例、容量、亲和力、预约和状态转换，不依赖 SQLx、Matrix SDK、Axum 或 UI。
- `application::rooms::joining` 编排“短期预约 → Matrix 加入 → 数据库确认”，失败时执行闭合补偿；`provisioning` 独立编排 Space 与实例供给；`entry` 作为 Facade 对调用方隐藏两阶段细节。
- `postgres-adapter::rooms` 分成目录、分配、解码和供给模块。候选选择在持有行锁的短事务内完成，领域评分函数不含 SQL 或基础设施分支。
- `matrix-adapter` 只把应用端口映射为标准 Client-Server API：创建 `m.space`、稳定别名解析、加入/离开和 `m.space.child` 状态。业务层不知道 Ruma JSON 类型。
- HTTP/Web 传输与全幅场景仍属于任务 19–20；本任务交付可由控制平面或 Bridge 组合的应用用例与真实基础设施适配器，没有用假路由或假 UI 冒充完成度。

## 2. 目录、评分与容量

- 一个主题大厅对应一个 `room_catalog_entry` 和一个 Matrix Space；每个可视化分片对应独立 `room_instance`。
- 公共目录按状态、可见性、语言和地区查询，并汇总活跃实例数、在线 Agent 投影与活跃度，不把 Matrix 成员投影误当授权真相。
- 自动分配顺序由领域策略集中决定：恢复原实例、好友同房、明确邀请、语言、地区、活跃度和容量。新增策略只扩展候选证据与评分，不在 HTTP 或 SQL 中复制分支链。
- 默认软阈值 180、硬阈值 250。自动模式在没有社交锚点且达到软阈值时要求新分片；手动切换可以越过软阈值，但永远不能突破硬阈值。
- `allocated_slots` 是并发分配事实；Matrix `member_count_projection` 只用于展示与对账。预约、槽位递增和唯一归属在同一数据库事务内提交。

## 3. 加入 Saga 与失败补偿

- 新加入先创建 `reserved` 预约，再在数据库事务外加入 Matrix；成功后转为 `committed`，失败则转为 `released` 并归还槽位。
- 确认失败时先离开 Matrix，再释放预约。任何一步补偿失败都保留原始故障与补偿故障，不吞错、不返回虚假“已加入”。
- 已有 `committed` 归属走幂等 Matrix 加入，不重复占槽；手动切换在确认新归属时原子释放旧归属。
- 未确认预约使用短期限与有界回收扫描。过期扫描只处理 `reserved`，不会误删已确认归属。
- `EnterLobbyService` 在无可用实例时自动触发供给并重新预约。并发 worker 正在建房时返回精确 `retry_at`；新容量被其他请求抢走时返回 `CapacityChanged`，不会把供给完成冒充成成员加入完成。

## 4. 可恢复自动供给

- Space 和实例都使用确定性安全别名。创建请求明确区分普通会话与 `m.space`，冲突或未知提交先按别名对账，绝不盲目重复创建。
- `room_provisioning_job` 持久化任务、目标、别名、Matrix Room ID 断点、租约与失败阶段。同一目录只允许一个待完成 Space，同一地区只允许一个待完成实例任务。
- 租约 ID 是 fencing token。租约过期或失败释放后，新 worker 接管原任务和已有断点；旧 worker 的 checkpoint、完成和释放全部被数据库拒绝。
- 实例流程严格为“创建/恢复 Matrix Room → 保存断点 → 写入 `m.space.child` → 原子发布 Active 实例”。Space 挂载失败时实例不会进入可分配目录。
- 建房完成、目录 Space 更新与实例发布都在各自短事务内完成。系统没有伪造跨 PostgreSQL/Matrix 的两阶段提交。

## 5. 验收结果

| 验收项 | 结果 |
| --- | --- |
| 领域评分 | 恢复与好友优先；无社交锚点时软阈值触发新分片 |
| 硬容量并发 | 8 个并发请求争抢 3 个槽位，仅 3 个成功，数据库计数不超卖 |
| 手动切换 | 新实例确认与旧归属释放原子完成 |
| 预约回收 | 只回收过期 `reserved`，槽位准确归还 |
| Matrix 加入失败 | 预约变为 `released`、槽位回到 0、用例返回失败 |
| 并发建房 | 两个 worker 只产生一个待办任务，另一方得到明确重试时间 |
| fencing | 过期租约接管保留 Matrix 断点，旧租约无法提交 |
| 断点续作 | 已保存 Room ID 时不重复创建，继续幂等挂载与发布 |
| Space 挂载失败 | 任务可接管，实例不发布，目录无虚假可用房间 |
| 空目录纵向流程 | 一次 `enter` 完成 Space、实例、挂载、预约、Matrix 加入和确认 |
| 真实 Synapse | 独立会话同步到 `m.space` 创建类型、稳定别名和 `m.space.child` 的 `via/suggested` |

## 6. 质量门禁

- `just check` 全量通过：Rust/TypeScript 格式、Clippy `-D warnings`、全目标全特性检查、构建、工作区测试、协议生成一致性、Secret 扫描和 GitHub Actions 固定版本检查。
- `python tools/database.py test` 使用隔离的真实 PostgreSQL 跑通 27 个测试，包括并发容量、手动切换、过期回收、供给 fencing、断点接管和完整进入 Saga。
- `python tools/matrix.py test` 的 2 个真实 Synapse 场景全部通过；大厅验收覆盖标准房间生命周期、Space 类型、别名解析、子房间状态、同步、回执和回填。
- Matrix 行为依据 [Matrix Client-Server API v1.15](https://spec.matrix.org/v1.15/client-server-api/)；`m.space.child` 通过 `matrix-rust-sdk` 的强类型 Ruma 内容生成，没有手写松散 JSON 协议。

## 7. 明确边界

- `member_count_projection` 仍是最终一致投影；它不能授权访问，也不能替代 `allocated_slots` 的事务容量账本。后续投影偏差修复属于运行维护与任务 39 的容量验收。
- 当前自动分片按可选地区提示创建实例，不承诺云厂商级地理路由。真实延迟测量、跨区域部署与联邦容量属于任务 37、39 和 40。
- 私人房间、邀请策略与 E2EE 分别属于任务 25、29 和 27。本任务的公共大厅 Space 不应被错误复用为私人信任边界。
- Web API DTO、浏览器状态机和 PixiJS 大厅场景在任务 19–20 接入本任务的目录与 `enter` 用例；在此之前不会添加无认证的临时 HTTP 捷径。
