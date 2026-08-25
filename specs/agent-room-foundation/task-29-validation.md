# 任务 29 验证记录：有界自动发言授权

## 1. 结论

任务 29 已完成。自动发言不是 Agent 在线后的隐式能力，而是一份默认不存在、范围精确、可撤销并且每次使用都重新校验当前权限的授权。授权按房间、Agent、可选精确实例、消息类别、受众、频率、总量、期限和风险扫描策略建模；创建与撤销均要求近期认证。

任何授权事实、产品设备状态、Agent 实例状态、Matrix 成员关系、Matrix 发送权限或风险扫描结果不确定时，请求都会失败关闭。Bridge 在读取本地声明、上传附件或向 Matrix 发送之前取得单次授权结果，因此拒绝路径不会遗留外部副作用。

## 2. 授权模型与硬边界

领域层定义独立的 `AutomationGrant` 聚合与值对象，不依赖 HTTP、PostgreSQL、Matrix 或 UI。授权包含：

- 唯一房间、唯一 Agent，以及可选的唯一 Agent 实例。
- 允许的消息类别集合与受众集合。
- 每分钟上限、授权期内总量上限和绝对到期时间。
- 是否必须通过风险扫描。
- 创建者、创建时间、撤销时间和当前消费计数。

系统默认没有授权，在线状态、设备验证或房间成员身份都不能自动生成授权。领域层还施加不可由 UI 放宽的硬上限：每分钟最多 60 次、授权期最多 10,000 次、最长 30 天。新增消息类别或受众只能通过显式注册扩展，核心授权流程不维护不断增长的条件分支。

## 3. 每次发送的权威校验顺序

应用用例按固定顺序处理自动发言：

1. 读取授权并确认未撤销、未到期。
2. 比对 Agent、可选精确实例、房间、消息类别和受众。
3. 校验当前分钟窗口、累计额度和幂等请求。
4. 重新读取产品设备与 Agent 实例的当前授权状态。
5. 重新读取 Matrix 当前成员关系与发送权限。
6. 按授权要求执行风险扫描。
7. 在数据库事务中原子消费额度。

权限变化不会等待授权过期：Agent 实例撤销、产品设备撤销、被踢出房间或 Matrix 发送权限降低后，下一次自动发言立即失败。控制平面、权限读取或风险扫描不可用时不会回退到缓存猜测。

## 4. PostgreSQL 原子消费与审计

迁移 `202608250003_automation_authorization.sql` 增加授权、消费和拒绝记录。消费事务对授权行加锁，并在同一事务内完成到期、撤销、分钟频率、累计额度与幂等校验：

- 并发请求不能突破频率或总量。
- 同一幂等键只消费一次；改写请求声明会被识别为篡改。
- 撤销与后续消费按数据库顺序收敛，撤销提交后不再放行。
- 拒绝审计只记录授权标识、结构化原因和时间，不保存消息正文。

真实 PostgreSQL 测试证明了并发限流、幂等、额度耗尽、撤销，以及每次发送重新读取当前产品权限。

## 5. 控制平面与 Bridge

控制平面公开以下接口：

| 方法与路径 | 行为 |
| --- | --- |
| `GET /automation-grants` | 列出当前主体可管理的自动发言授权 |
| `POST /automation-grants` | 在近期认证与精确同源保护下创建授权 |
| `DELETE /automation-grants/{grant_id}` | 撤销授权，不允许静默扩大或复活 |
| `POST /automation-grants/{grant_id}/authorizations` | 由持有设备密钥的 Agent 实例申请一次发送许可 |

创建和撤销使用浏览器会话边界；单次发送授权使用签名设备请求，并绑定请求体、时间与防重放信息。所有响应禁止缓存，未知或畸形载荷不会被当作成功。

Bridge 的自主消息命令必须携带授权 ID。它先向控制平面申请许可，再执行本地声明读取、内容上传和 Matrix 发送。适配器把网络失败、非预期状态码和 Schema 错误映射为类型化拒绝；不存在“授权服务坏了就先发出去”的危险回退。成功事件写入 `autonomous_agent` 来源，人工发送仍保留原来源。

## 6. Inspector 与来源可见性

Web 端使用独立 `features/automation` 功能模块，领域 Schema、HTTP Adapter、TanStack Query 状态和 UI 组件分层。房间信标提供 Automation Inspector：

- 默认关闭且初始授权数为零。
- 只能选择当前主体真实拥有的 Agent 和活跃实例，不提供伪造选项。
- 精确展示房间、实例、消息类别、受众、频率、总量、期限和风险扫描影响。
- 用户确认影响并完成近期认证后才能创建；撤销同样要求近期认证。
- 服务端状态加载失败时明确失败关闭，不展示乐观成功。
- 桌面使用侧边 Inspector，窄屏使用全屏布局；关闭后恢复触发按钮焦点。
- 消息预览和正文检查器对自主消息显示 `AUTO`／`自动` 标记与完整来源信息。

英文与简体中文文案均进入类型化消息目录。加载、空状态、错误、成功和屏幕阅读器播报均有明确状态；持续动画尊重 `prefers-reduced-motion`。

## 7. 验证证据

### TypeScript、构建与浏览器

```text
pnpm check
  Prettier、ESLint、TypeScript、协议一致性通过
  54 个 Vitest 文件、201 个测试通过

pnpm --filter @agent-room/web build
  Web 生产构建通过

pnpm --filter @agent-room/web exec playwright test e2e/automation-grants.e2e.ts
  2/2 通过
  覆盖默认关闭、范围配置、近期认证、创建、撤销、来源标记和 390px 布局
```

浏览器证据：

- `artifacts/browser/task-29/automation-grant-desktop.png`
- `artifacts/browser/task-29/automation-grant-mobile.png`

### Rust 与真实 PostgreSQL

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
  全部通过
  覆盖领域范围、到期、撤销、频率、控制平面、Bridge 顺序和来源

.venv/Scripts/python.exe tools/database.py test
  真实 PostgreSQL 全部通过
  automation 2/2 通过；全套数据库集成测试通过
```

Windows MSVC 的本地化链接器提示和第三方 `proc-macro-error2` 未来兼容提示不是门禁失败。

## 8. 变更提交

- `65104a7`：定义有界自动发言授权领域模型。
- `308de6e`：实现失败关闭的自动授权应用流程。
- `1fb3f39`：持久化授权、原子消费与拒绝审计。
- `e19396c`：公开自动授权控制平面 API。
- `0a699ca`：Bridge 在任何发送副作用前强制授权。
- `a6ed944`：增加 Web 自动授权网关。
- `c17f1c3`：完成 Automation Inspector、近期认证与来源标记。
- `07a4831`：修复全量验收暴露的动画可见性竞态测试。

## 9. 下一步

下一项是任务 30：屏蔽、举报与房间治理。实现必须区分本地立即屏蔽、服务端投递阻断、Matrix 房间权限和显式举报证据；管理员不能借治理能力获得 E2EE 私聊解密后门。
