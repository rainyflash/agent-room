# 任务 12 Agent 注册、归属与 Matrix 身份验证记录

> 验证日期：2026-08-24
>
> 结论：通过
>
> 对应任务：[实施计划 12](./tasks.md#m1内部纵向切片)
>
> 核心实现提交：`3fd6049`、`1367bef`、`53ff289`、`2fec578`、`6155a3d`、`af248b8`、`c3e6f8b`、`d64474f`、`3ecaf45`

## 1. 架构边界

- 领域层分别建模 `Agent`、`AgentMemberships`、`AdapterBinding` 和 `AgentInstance`。Agent 生命周期不依赖成员集合，Owner/Operator/Viewer 权限与最后 Owner 保护由纯领域规则维护，不依赖 HTTP、SQLx 或 Matrix SDK。
- 应用层通过创建回执、Agent 仓储、成员仓储与事务、实例登记事务、Matrix 身份签发、密钥摘要、时钟和标识工厂等端口编排用例。创建、绑定和转移规则不进入 Axum Handler 或 PostgreSQL 查询。
- PostgreSQL 是 Agent 归属、设备绑定、公钥和幂等回执的权威事实源。成员变更、实例登记和对应 Outbox 事件在同一事务提交；高风险校验直接读取权威表，不依赖异步投影。
- `matrix-adapter` 只实现 Matrix Application Service 身份端口。控制平面组合根注入该实现；应用与领域层不知道 Application Service Token、HTTP 端点或 Synapse 类型。
- HTTP 层只负责同源会话、近期认证、设备证明、UUIDv7、Base64、正文大小和 DTO 映射。所有响应使用结构化错误、关联 ID 与 `Cache-Control: no-store`。

## 2. 身份、归属与角色约束

- 创建 Agent 时先用幂等请求预留 UUIDv7 Agent ID，再由该 ID 确定性派生 `_agent_<32位小写十六进制>` Matrix Localpart。用户主体 Matrix User 与 Agent Matrix User 永远不是同一身份。
- 创建完成事务同时写入 Agent、首个 Owner、创建回执和 `agent.registered.v1` Outbox 事件。相同请求恢复同一 Agent，不生成第二个身份。
- 只有 Owner 可以增加、调整或撤销成员；Operator 可以注册实例但不能管理成员；Viewer 只能查看，不能注册实例或转移归属。
- 成员事务锁定活跃 Agent 和完整成员集合，再运行领域规则。最后一个活跃 Owner 无法被降级或撤销；先增加第二个 Owner 后才能完成控制权转移。
- 重复授予相同角色或重复撤销已撤销成员不制造多余状态与 Outbox 事件。

## 3. Agent Instance 与防冒充

- 实例注册必须来自未过期、未撤销、已验证且属于当前主体的 Agent Room Device；随后再次确认主体是目标 Agent 的 Owner 或 Operator。
- 每个实例绑定 Agent、用户设备、Adapter Binding、32 字节 Ed25519 实例公钥和确定性 Matrix Device ID。Matrix Device ID 从稳定 Agent Instance ID 派生，重试不会漂移。
- 活跃实例公钥在整个服务范围唯一；同一 Agent、设备和 Adapter Binding 也只有一个活跃实例。另一主体、Agent 或设备复用公钥时事务返回冲突，不会部分写入。
- 设备撤销事务同时撤销设备 Token Family 并把关联 Agent Instance 标为 `revoked`。后续控制面请求不能继续代表该设备登记或改变实例。
- `externalSubjectHash` 只保存固定 32 字节不可逆摘要，不保存 Codex、A2A 或其他宿主的原始账号标识。

## 4. Matrix Application Service 边界

- Synapse 注册文件只把 `^@_agent_[0-9a-f]{32}:matrix\.agent-room\.localhost$` 用户命名空间交给本项目 Application Service，普通用户与既有 Matrix 身份不在其控制范围。
- 控制平面通过标准 Application Service 注册与登录流程幂等建立 Agent User 和指定 Device 会话，不读写 Synapse 数据库或内部缓存。
- Homeserver 返回的 User ID 和 Device ID 必须与应用请求完全一致；任一不一致都视为内部协议破坏，凭据不会交给 Bridge。
- Application Service Token 只从 `AGENT_ROOM_MATRIX_APPSERVICE_TOKEN` 进入控制平面出站适配器。配置调试输出脱敏，PostgreSQL 不保存该 Token，Agent HTTP 响应也只含目标 Agent Device 的会话凭据。
- 实例重复登记可以重新签发 Matrix 会话 Token，但稳定 Agent、Binding、Instance 和 Device ID 不变。明文 Token 不作为幂等键或业务主键。

## 5. HTTP 与数据边界

| 场景 | 结果 |
| --- | --- |
| 创建 Agent 缺少精确前端 Origin、主机会话或 UUIDv7 幂等键 | 在业务用例前拒绝 |
| 成员变更只有普通活跃会话 | 要求近期认证，不静默降级 |
| 实例请求缺少 Bearer、设备 ID、时间、Nonce 或 64 字节签名 | 在设备认证和 Agent 用例前拒绝 |
| 设备证明搬到另一方法、路径、正文或 Token | 规范消息或签名校验失败 |
| Agent ID、Principal ID、请求 ID 不是 UUIDv7 | 返回稳定校验错误 |
| 实例公钥或主体摘要不是 32 字节 | 拒绝进入应用用例 |
| 未注册适配器 Schema 时提交非空 `configuration` | Fail-closed，拒绝持久化任意 JSON 或凭据 |
| 成功返回 Agent Device Token | 强制 `no-store` 与 `no-referrer`，不包含 Application Service Token |

HTTP 请求正文限制为 64 KiB，DTO 使用 `deny_unknown_fields`。任务 14 提供按适配器类型注册的配置 Schema 前，空对象是唯一允许的 Adapter 配置；用字段名黑名单猜测秘密不是安全方案，因此没有采用。

## 6. 幂等、事务与故障语义

- `agent_creation_request` 在调用 Matrix 前预留稳定 Agent ID。Matrix 暂时不可用时请求保持可重试；重试继续对账同一 Localpart，而不是制造第二个 Matrix User。
- 完成创建时，数据库约束触发器要求 `completed` 回执必须引用真实 Agent。Agent、首个 Owner、Outbox 和完成状态在同一事务提交。
- `agent_instance_registration_request` 与 Adapter Binding、Agent Instance 和 Outbox 在同一事务提交。相同 ID、主体、设备和请求指纹返回原登记；篡改任一绑定字段返回冲突。
- Matrix 无法与 PostgreSQL组成分布式 ACID 事务。实现采用稳定标识 + 幂等对账，不伪造两阶段提交；Matrix 会话签发失败时数据库中的稳定实例可由同一请求安全恢复并重新签发会话。
- 创建请求完成后再次提交直接返回数据库结果，不重复访问 Matrix。实例会话凭据不落 PostgreSQL，因此重试只重新签发凭据，不泄漏旧 Token。

## 7. 真实服务与攻击验收

| 验收项 | 结果 |
| --- | --- |
| 真实 PostgreSQL Agent 创建幂等与正文篡改 | 同请求返回同 Agent，篡改返回冲突 |
| 真实 PostgreSQL 成员并发与最后 Owner 保护 | 只有 Owner 可变更，最后 Owner 保留 |
| 真实 PostgreSQL 实例绑定与公钥冒用 | 合法 Owner/Operator 成功，跨主体与跨 Agent 冒用失败 |
| Viewer 注册实例 | 在数据库写事务和 Matrix 签发前失败 |
| Matrix 返回错误 User/Device 身份 | 应用拒绝下发凭据 |
| 真实 Synapse 独立 Agent User | 幂等创建成功，和用户主体身份分离 |
| 真实 Synapse Agent Device 会话 | 指定 Device ID 登录成功，返回身份完全一致 |
| 控制面真实依赖逐层断连 | PostgreSQL、Matrix 与对象存储分别映射到准确降级层 |

`python tools/database.py test` 在隔离测试数据库运行 15 个真实 PostgreSQL 场景并全部通过，结束后自动删除测试库。`python tools/matrix.py` 的 2 个真实 Synapse 场景全部通过。`python tools/control-plane.py test` 的真实依赖健康与断连场景通过。

## 8. 质量门禁与明确边界

- `just check` 通过 Rust/TypeScript 格式、Clippy `-D warnings`、类型检查、构建、普通测试、协议一致性、Secret 扫描和 GitHub Actions 固定版本检查。
- `just coverage` 合并普通测试、真实 PostgreSQL 和真实 Synapse 后，Rust 总行覆盖率为 79.34%，高于 60% 门禁；Agent 领域、Agent 应用、Agent HTTP、Matrix 身份签发、PostgreSQL 创建/实例/成员适配器行覆盖率分别为 77.28%、73.97%、84.87%、86.74%、88.84%、86.74% 和 88.66%。TypeScript 四项覆盖率均为 100%。
- `cargo deny check` 的 advisories、bans、licenses 和 sources 通过；Node 官方审计端点未发现已知漏洞。本任务只复用工作区已有 Matrix 适配器，没有引入第二套 Matrix 客户端或身份 SDK。
- 本任务没有实现 Bridge 常驻 Matrix Store、实例在线租约、Agent Card Schema、房间消息、E2EE 或客户端界面；分别属于任务 13–15、18、27 和 19–22。验证记录不把“能够签发身份”冒充成“完整 Agent 已经上线”。
- Application Service 注册、独占用户命名空间与代表虚拟用户调用 Client-Server API 的依据来自 [Matrix Application Service API](https://spec.matrix.org/latest/application-service-api/)；Agent Device 登录与标识校验依据 [Matrix Client-Server API](https://spec.matrix.org/latest/client-server-api/)。
