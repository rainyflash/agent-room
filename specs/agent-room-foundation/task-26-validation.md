# 任务 26 验证记录：直接会话

## 1. 结论

任务 26 已完成。直接会话不是浏览器里的临时抽屉，而是由领域策略、应用用例、PostgreSQL 权威关系、真实 Matrix DM 与 Web 会话坞共同组成的纵向切片。当前实现覆盖：

- 从 Agent Inspector 创建双方唯一的直接会话；重复发起复用同一产品记录和 Matrix Room。
- 由受管目标 Agent 代表自己建立邀请制 Matrix DM，并在双方账户写入各自的 `m.direct` 映射。
- 继续使用任务 21 的消息预览、按需正文、发送器和内容授权，不创建第二套消息时间线。
- 使用 Matrix 增量同步与回填恢复离线期间的消息；浏览器 URL 只保存当前会话标识，不保存消息副本。
- 显示消息后发送 `m.read.private`，不把私信阅读位置广播成普通公开回执。
- 屏蔽事实同时收紧控制平面投递与客户端 `m.ignored_user_list`；解除屏蔽按相反顺序恢复。
- 在线状态只呈现粗粒度可用性；任一方向屏蔽后直接隐藏，不暴露精确时间戳或最后活动时间。
- 桌面端提供沉浸式全幅会话坞和持久会话轨道；390px 窄屏保持完整可操作且无横向溢出。

任务 27 才负责 Matrix E2EE、设备验证、交叉签名、密钥备份和恢复。本任务只建立了私密访问控制与正确协议语义，没有把“私密房间”冒充“端到端加密”。

## 2. 架构与事实源

```text
Agent Inspector / 直接会话轨道
  -> DirectSessionCoordinator
      -> Control Plane HTTP Adapter
          -> DirectSessionUseCases
              -> DirectSession / DirectContactPolicy
              -> DirectSessionStore -> PostgreSQL
              -> DirectSessionMatrixProvisioner -> Synapse
      -> Matrix Web Adapter
          -> 加入、m.direct、m.ignored_user_list、m.read.private
  -> 既有 MessageLayer
      -> Matrix Timeline + Content API
```

| 事实 | 唯一权威源 | 禁止的替代实现 |
| --- | --- | --- |
| 双方唯一会话、生命周期和屏蔽策略 | PostgreSQL | React 状态、Local Storage 或 Matrix 别名反推 |
| 房间成员、时间线、账户数据和回执 | Synapse | 业务库建立第二套私信表 |
| 当前打开的会话 | URL `direct` 查询参数 | 多个组件各自同步一份选择状态 |
| 正文读取权限 | 既有内容授权与当前 Matrix 成员状态 | DM UI 直接绕过内容票据 |

领域层不依赖 Axum、SQLx、Matrix SDK 或 React。应用层只依赖目录、仓储、时钟、标识工厂和 Matrix 供给端口；控制平面与浏览器只负责组合具体适配器。

## 3. 创建、复用与关系恢复

`POST /direct-sessions` 的语义是“创建或复用”，不是无条件建房：

1. 先按 `(principal_id, target_agent_id)` 查询既有关系。
2. 首次建联必须通过当前 Agent 可发现性检查。
3. 已存在关系使用“已知联系人”读取，即使对方后来转为私密或退役，历史会话仍可列举、检查、屏蔽和解除屏蔽。
4. 新关系以冻结目录和 `provisioning` 会话预留唯一记录。
5. 以稳定别名让受管目标 Agent 创建邀请制、`is_direct: true` 的双人 Matrix Room。
6. Matrix 成功后才激活产品目录与房间实例；未知提交或持久化冲突不会被伪装成成功。

“当前可发现目标”和“已有关系中的已知目标”被拆成两个目录端口，避免为了恢复旧会话而重新暴露不可发现 Agent，也避免首次联系绕过隐私设置。

## 4. 屏蔽、阅读状态与在线隐私

屏蔽不是纯 UI 开关：

- 屏蔽时先持久化产品事实，再写入 Matrix 忽略列表；即使客户端同步失败，服务端也已停止新的内容投递。
- 解除时先移除 Matrix 忽略，再撤销产品屏蔽，避免服务端已放行而客户端仍静默丢弃的窗口。
- 任一方向存在屏蔽时，`DirectContactPolicy.delivery_allowed()` 为假，新内容授权明确拒绝；历史内容仍按原授权读取，不伪造密码学删除。
- 被屏蔽联系人在线状态为 `hidden`；未屏蔽时也只提供 `coarse`，不返回精确最后在线时间。
- 会话面板只对当前展示的最新真实事件发送 `m.read.private`；找不到对应事件时响亮失败，不伪造已读。

当前公开 HTTP 入口用于 Principal 屏蔽 Agent；仓储与策略能够读取双向屏蔽事实。Agent 主动发起的屏蔽命令尚未暴露为独立产品入口，不能在文档里假装已经存在。

## 5. Control Plane 与 Web 体验

Control Plane 提供同源、受认证、`no-store` 且有 8 KiB 正文上限的接口：

- `GET/POST /direct-sessions`
- `GET /direct-sessions/{catalog_id}`
- `PUT /direct-contacts/{agent_id}/block`

非法 Origin、UUID、JSON、会话状态和联系人会返回稳定错误码；服务端不会信任客户端提交的 Matrix 用户标识。

Web 按功能拆分领域类型、Zod 边界、控制平面/Matrix 适配器、协调器、TanStack Query 状态和 UI。会话坞复用既有消息层，直接会话默认展开正文预览但仍需显式读取受保护正文；Agent Inspector 的打开与屏蔽操作都有 pending、失败和重试状态。会话轨道不是第二套路由器，当前会话由 URL 单一事实源驱动。

浏览器验收覆盖：打开既有会话、按需读取正文、屏蔽与解除、从 Agent Inspector 新建会话、桌面沉浸式布局和 390px 移动布局。证据截图：

- `artifacts/browser/task-26/playwright-direct-session.png`
- `artifacts/browser/task-26/playwright-direct-session-mobile.png`

## 6. 真实 PostgreSQL 证据

`tools/database.py test` 在隔离数据库执行全部 41 个真实 PostgreSQL 用例，其中直接会话用例验证：

1. 相同双方重复预留只返回同一会话。
2. 激活状态、Matrix Room 和版本跨仓储读取完整恢复。
3. 屏蔽、重复屏蔽和解除屏蔽均幂等往返。
4. 目标转为私密、退役且所有权撤销后，不再满足首次联系查询，但仍能通过已有会话恢复为已知联系人。
5. 数据库唯一约束和生命周期约束拒绝伪造关系。

测试脚本自动创建、迁移并销毁隔离数据库，没有复用进程内假仓储。

## 7. 真实 Synapse 证据

`tools/matrix.py test` 的 4 个真实 Synapse 用例全部通过。任务 26 新增用例验证：

1. 受 Application Service 管理的 Agent 能代表自己创建邀请制双人 DM。
2. 相同稳定别名重复创建解析回同一 `room_id`。
3. Agent 的 `m.direct` 账户数据精确指向对端与该房间。
4. 创建者同步到 `joined`，对端先同步到 `invited`，接受后同步到 `joined`。
5. 真实消息写入成功，对端可提交 `m.read.private`。

首次真实验收曾错误地让不在 DM 中的 Application Service 发送者读取成员状态，Synapse 正确返回 `403`。测试随后改为由被邀请客户端通过同步验证邀请，没有放宽服务端权限。这是保留安全边界的修正，不是绕过失败。

## 8. 质量门禁

```text
just check
  Rust fmt、Clippy -D warnings、workspace 全目标/全特性检查与测试通过
  41 个 TypeScript 测试文件、144 个测试通过
  TypeScript strict、ESLint、生产构建、协议生成/一致性通过
  506 个文本文件 Secret 扫描、11 个 Actions 固定引用通过

just web-browser
  15 个真实 Chrome 用例通过
  覆盖直接会话、私人房间、消息、交接、200 Agent 场景与移动端回归

.venv\Scripts\python.exe tools\database.py test
  真实 PostgreSQL 全套 41 个集成用例通过

.venv\Scripts\python.exe tools\matrix.py test
  真实 Synapse 4/4 通过
```

构建仅保留既有 Matrix Crypto WASM、字体和主包的大分块提示，以及 Windows 本地化链接器输出与第三方 `proc-macro-error2` 未来兼容提示；严格 Clippy、ESLint、类型与测试门禁均为零失败。

## 9. 明确边界与下一步

- 当前 DM 是 Matrix 私密房间，不是强制 E2EE；任务 27 必须在无法建立安全设备会话时拒绝回退明文。
- 当前只展示粗粒度在线状态，不提供精确最后在线时间；这不是缺失，而是隐私策略。
- 当前屏蔽停止新投递并同步忽略列表，不声称能撤回对端已经收到的历史明文。
- 下一步是任务 27：Matrix E2EE、设备验证、交叉签名、密钥备份与多设备恢复。
