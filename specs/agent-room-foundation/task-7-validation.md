# 任务 7 数据库与仓储验证记录

> 验证日期：2026-08-23
> 结论：通过
> 对应任务：[实施计划 7](./tasks.md#m1内部纵向切片)
> 核心实现提交：`d85b245`

## 1. Schema 与事实边界

- 首个只向前 SQLx 迁移创建 19 张 Agent Room 自有表，覆盖 Principal、Device、Agent、Ownership、Instance、Adapter、Room、Content、Handoff、Grant、Moderation、Audit、Outbox 和 Matrix Projection。
- Matrix 时间线仍由 Synapse 持有；业务库只存产品领域实体、内容引用和可重建投影，没有复制第二套可写消息表。
- UUIDv7、长度、状态、时间顺序、JSON 对象、摘要尺寸和乐观版本均有数据库检查约束；引用关系由外键保护。
- 活跃 Agent 至少保留一个未撤销 Owner，该规则由延迟约束触发器在事务提交时验证，允许 Agent 与首个 Owner 原子创建。
- 审计事件只允许追加；运行时角色没有更新/删除权限，数据库所有者也会被不可变触发器拒绝。

## 2. 角色、迁移与依赖方向

- `agent_room` 只用于迁移，`agent_room_runtime` 只用于应用 DML；两者都是非超级用户，运行时角色没有 Schema `CREATE` 权限。
- 控制平面只读取 `AGENT_ROOM_DB_RUNTIME_PASSWORD`，不会把迁移凭据装入应用配置。
- SQLx 迁移会话显式固定 `search_path=public`，避免同名 `agent_room` Schema 出现后把 `_sqlx_migrations` 解析到另一位置并错误重放迁移。
- SQL 只存在于 PostgreSQL 基础设施适配器；应用端口依赖 `PrincipalRepository` 和 `AgentRepository`，HTTP Handler 没有直接数据库查询。
- Principal 与 Agent 更新使用 `WHERE version = expected` 比较并交换；冲突、约束、缺失、不可用和损坏数据映射为稳定仓储错误，不向应用层泄漏数据库原始错误。

## 3. 真实 PostgreSQL 验收

`python tools/database.py test` 每次强制重建名称固定的隔离测试库，运行完成后强制断开连接并删除测试库。4 个真实数据库测试全部通过：

| 验收项 | 结果 |
| --- | --- |
| 从空库执行迁移并再次执行 | 19 张表完整，第二次迁移不重放 |
| 运行时角色越权建表 | PostgreSQL `42501` 拒绝 |
| 两个写者使用同一聚合版本并发保存 | 恰好一个成功，另一个稳定映射为 `Conflict` |
| Agent 与 Owner 事务创建 | 同时提交并可通过仓储恢复 |
| 不存在的 Owner 创建 Agent | 外键失败，Agent 插入完整回滚 |
| 插入孤儿 Device | PostgreSQL `23503` 拒绝 |
| 显式事务回滚 | 回滚后主体记录不存在 |
| 运行时与所有者修改审计事件 | 分别由权限和不可变触发器拒绝 |

同一套本地基础设施下，`python tools/control-plane.py test` 仍通过 PostgreSQL、Matrix 和对象存储的正常/逐层断连矩阵，证明运行时角色拆分没有破坏健康模型。

## 4. 自动化与质量结果

- `tools/database.py` 统一处理旧开发卷的运行时角色补建、迁移、隔离测试库生命周期和覆盖率，凭据只通过环境或标准输入传递，不进入命令行输出。
- GitHub Actions 集成 Job 在真实依赖启动后执行数据库覆盖测试，再迁移主业务库并运行既有控制平面断连矩阵。
- Rust 覆盖率合并普通 Workspace 测试与真实 PostgreSQL 仓储测试后，行覆盖率为 66.67%，通过不低于 60% 的门禁；不会通过排除基础设施适配器粉饰数字。
- `cargo fmt --check`、Clippy `-D warnings`、Workspace 类型检查、单元测试和真实数据库测试全部通过。
- `cargo deny check` 的 advisories、bans、licenses 和 sources 全部通过；`pnpm audit` 未发现高危漏洞。

## 5. 已知非阻断项

- Windows MSVC 仍会输出本地化的 `linker_messages` 提示；它不影响编译、测试或覆盖率结果。
- 首版只实现当前应用端口已经声明的 Principal 与 Agent 仓储；其他表的专用端口随对应功能任务增加，禁止为了“仓储齐全”提前制造无用 CRUD。
