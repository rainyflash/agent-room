# 任务 30 验证记录：隐私受限的房间治理

## 1. 结论

任务 30 已完成。用户屏蔽、服务端私信投递阻断、举报案件、Matrix 治理副作用和追加式审计是五个明确边界，不是一个暧昧的“删除消息”开关。本地屏蔽在远端请求完成前立即生效；后续私信投递同时由服务端权威策略拒绝。

管理者不具有 E2EE 正文解密后门。举报默认只附带房间、Matrix 事件标识和加密标记；只有举报者主动勾选时，当前已显示的预览才作为证据提交。受保护正文的票据、下载和解密链路不会被举报触发。

## 2. 领域与权限模型

领域层独立建模以下事实，不依赖 HTTP、PostgreSQL、Matrix 或 React：

- `ModerationCase`：举报者、目标、结构化原因、必要说明、显式证据和单向案件状态。
- `ModerationAction`：隐藏、禁言、移出、封禁及其 pending、applied、failed、reversed 生命周期。
- `ModerationAuditEvent`：操作者、目标引用、原因、时间、结果、关联标识和可选房间，没有消息正文字段。
- `ModerationRole`：房间管理、平台治理和独立审计读者彼此分离。

应用用例每次重读当前房间成员与管理权限；不信任 UI 传入的角色。房间案件队列只在当前角色仍可治理时返回，并严格按 `room_catalog_id` 隔离。审计读取使用独立角色，审计读者不因此获得治理写权限。

## 3. 一致性与 Matrix 副作用

治理动作采用可诊断的三阶段顺序：

1. 在 PostgreSQL 中预留 pending 动作并追加请求审计。
2. 在 Matrix 执行服务端副作用。
3. 将动作终态写为 applied 或 failed，并追加结果审计。

因此 Matrix 失败不会伪装成成功，崩溃后也有持久化事实可对账。隐藏使用 `io.github.rainyflash.agentroom.moderation.notice.v1` 当前状态，不销毁原事件；撤销写入 `hidden: false`，Web 投影可恢复原预览。禁言通过 Matrix power levels 收紧发言权；移出与封禁分别走 kick 和 ban，撤销走 invite 或 unban + invite。

## 4. 存储与滥用防护

迁移 `202608250004_moderation.sql` 增加案件、动作、平台操作者角色和追加式审计。真实 PostgreSQL 验证了：

- 并发举报共享同一限速窗口，不能被多副本绕过。
- 运行时数据库角色不能授予自己平台治理或审计权限。
- 审计表触发器拒绝运行时修改和删除旧事件。
- 房间案件查询不跨房，案件证据未提交的摘录以 `NULL` 保存。

## 5. HTTP 与 Web 产品边界

控制平面提供举报、本人案件、房间案件、房间动作、撤销和审计接口。写入要求精确同源，创建案件与动作使用 UUIDv7 幂等标识；治理与撤销还要求近期认证和显式影响确认。所有响应禁止缓存，请求正文有 16 KiB 硬上限。

Web 端按功能模块分离领域 Schema、网关、TanStack Query 和视图：

- 消息 Inspector 中的举报对话框默认不附带预览，不读取正文。
- 房间治理入口只在至少一项当前管理查询获权时显示。
- 治理工作台同时展示案件、动作和授权审计；证据区明确区分“无明文摘录”与“举报者显式摘录”。
- 用户屏蔽先写入浏览器本地阻断集合，再并发同步服务端与 Matrix ignore 事实；远端失败不回滚本地安全效果。
- 英文和简体中文文案均进入类型化资源，对话框支持 Escape、点击遮罩关闭、焦点恢复和 reduced-motion。

## 6. 验证证据

### 静态门禁与全量自动测试

```text
pnpm check
  Prettier、ESLint、TypeScript、协议一致性通过
  57 个 Vitest 文件、214 个测试通过

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
  全部通过
```

Windows MSVC 的本地化链接器信息和第三方 `proc-macro-error2` 未来兼容提示不是门禁失败。

### 真实 PostgreSQL

```text
.venv/Scripts/python.exe tools/database.py test
  全套隔离 PostgreSQL 集成测试通过
  moderation 2/2 通过
```

覆盖原子限速、房间案件隔离、当前权限、动作终态、撤销、独立审计角色和追加式审计。

### 真实 Synapse

```text
.venv/Scripts/python.exe tools/matrix.py test
  5/5 真实 Synapse 测试通过
```

新增验收在真实受管 E2EE 房间中证明：隐藏状态可读、撤销后恢复；禁言后 Synapse 服务端拒绝成员发送，撤销后恢复。入房夹具也改为有界、只对可恢复故障生效的退避，Forbidden 仍会立即失败。

### 真实 Chromium

```text
pnpm --filter @agent-room/web exec playwright test e2e/moderation-governance.e2e.ts
  1/1 通过
```

验收覆盖举报前后正文票据与下载均为零、显式预览证据、案件进入当前房间、应用治理、撤销和无横向溢出。截图：`artifacts/browser/task-30/moderation-governance.png`。

## 7. 变更提交

- `fdcf42a`：建模可审计的治理领域。
- `9aa2d22`：编排失败关闭的治理用例。
- `e010d37`：持久化案件、动作、角色和追加式审计。
- `829e667`：将四类可逆治理映射到 Matrix。
- `68893a2`：公开受认证和同源保护的治理 API。
- `4aa7b3e`：使本地屏蔽立即生效。
- `abd336b`：增加 Web 治理网关与 Schema。
- `7609b94`：投影当前 Matrix 隐藏状态并支持恢复。
- `08907ac`：交付举报与房间治理工作台。
- `e196f8a`：增加真实 PostgreSQL、Synapse 与 Chromium 证据。
- `537d7b5`：清理投影链路并通过严格 Web 门禁。

## 8. 下一步

下一项是任务 31：Tauri 桌面壳与 Bridge 生命周期。实现必须保持窄化 Capabilities，WebView 不得获得文件系统、Shell 或 Process 通配权限；Bridge 崩溃必须可诊断且不能无限重启。
