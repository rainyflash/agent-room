# 任务 25 验证记录：私人房间完整生命周期

## 1. 结论

任务 25 已完成。私人房间不再是浏览器里的演示卡片，而是一条由领域聚合、应用 Saga、PostgreSQL 权威事实、真实 Synapse 房间和浏览器交互共同组成的纵向切片。当前实现覆盖：

- 创建隐藏的邀请制 Matrix Room，并用稳定别名处理重复创建。
- 邀请、接受、拒绝、退出、移除、封禁、权限变更、房主转移和归档。
- 查看、发言、邀请、管理与自动发送能力的产品策略；发言边界同步到 Matrix Power Levels。
- 仅向受邀或已加入的 Principal 返回私人房间，不从 Matrix 别名或时间线反推产品成员关系。
- 创建失败时停留在创建流程，不导航到伪造的房间场景。
- 房主客户端不在线时，房间和成员事实仍由 Synapse 与 PostgreSQL 持续维护。
- 被移除成员立即从产品授权和权威列表中消失；即使 Matrix 成员投影尚未收敛，受保护正文读取仍被拒绝。

任务 27 才负责 Matrix E2EE、设备验证和移除后的后续密钥轮换。本任务没有把服务端访问控制冒充密码学撤销。

## 2. 事实源与依赖边界

```text
Web UI
  -> 私人房间协调器
      -> Control Plane HTTP 适配器
          -> PrivateRoomUseCases
              -> PrivateRoom 领域聚合
              -> PrivateRoomStore 端口 -> PostgreSQL
              -> PrivateRoomMatrixGateway 端口 -> Synapse
      -> Matrix Web Gateway（接受、拒绝、退出的客户端成员动作）
```

边界被刻意拆成两类事实：

| 事实 | 唯一权威源 | 禁止的替代实现 |
| --- | --- | --- |
| 房间生命周期、房主、邀请和产品能力 | PostgreSQL 中的私人房间聚合快照 | 从别名、Timeline 或 UI 状态反推 |
| Matrix 成员、Power Levels 和通信事件 | Synapse | 在业务库建立第二套消息时间线 |
| 用户可读取受保护正文 | 产品能力与可信 Matrix 当前成员状态的交集 | 任一侧单独放行 |
| 浏览器当前房间 | 创建/加入成功后的协调器结果 | 先展示场景再后台补创建 |

领域层不依赖 Axum、SQLx、Matrix SDK 或 React。应用层只依赖端口，具体 PostgreSQL、Synapse 和浏览器实现由组合根注入，因此真实依赖测试与内存测试使用同一套业务规则。

## 3. 生命周期与 Saga 顺序

跨 PostgreSQL 和 Matrix 的操作不能伪装成原子事务。实现使用显式顺序降低越权窗口，并以稳定标识和幂等收敛处理重试：

| 操作 | 顺序与安全理由 |
| --- | --- |
| 创建 | 创建/复用隐藏 Matrix Room → 写入初始发言边界 → 持久化权威快照；重复幂等键必须收敛到同一房间 |
| 邀请 | Matrix 邀请成功 → 持久化产品邀请事实，避免产品先显示不可接受的邀请 |
| 接受 | 浏览器真实加入 Matrix → Control Plane 确认 Matrix 已为 `join` → 持久化已加入状态 |
| 拒绝、退出 | 浏览器完成 Matrix 离开 → Control Plane 确认已离开 → 持久化终态 |
| 移除、封禁 | Matrix `kick` / `ban` 先收紧边界 → 再保存产品终态 |
| 撤销发言 | Matrix 先降权 → 再保存产品权限，优先关闭越权窗口 |
| 授予发言 | 产品权限先保存 → Matrix 再升权，避免 Matrix 单边提前放权 |
| 转移房主 | 先撤销必要的旧房主能力 → 保存新房主事实 → 再授予新房主能力，并支持重复调用收敛 |
| 归档 | Matrix 先变为只读 → 产品聚合再归档 |

每个失败都保留操作名、失败阶段和稳定错误类别；未知提交状态不会被吞掉或伪装成成功。

## 4. Control Plane 与浏览器体验

Control Plane 提供以下受认证、同源校验且 `no-store` 的接口：

- `GET/POST /private-rooms`
- `GET/DELETE /private-rooms/{catalog_id}`
- 邀请、接受、拒绝和退出成员接口
- 移除、封禁和成员权限接口
- 房主转移接口

创建使用 `Idempotency-Key` 作为稳定 `RoomCatalogId`，房主转移和归档要求近期重新认证。所有 JSON 和路径标识都经过严格解析，非法 Origin、非法 UUID、未知成员和非法状态转换会明确失败。

Web 按功能组织了领域类型、Zod 边界、Control Plane/Matrix 适配器、协调器、TanStack Query 状态和 UI：

- 三步 Sheet 创建流程，支持初始邀请和能力配置。
- 权威房间列表与待处理邀请；不使用硬编码演示房间。
- 邀请、成员权限、移除、封禁、房主转移、退出和归档治理界面。
- 中英文资源、桌面与 390px 移动布局。
- 创建或加入失败时保留可恢复错误，不进入房间路由。

浏览器验收专门让创建请求失败，并断言页面没有导航到虚假房间；成功路径则断言服务端返回的真实 `catalogId` 和 `matrixRoomId` 被用于后续加入和导航。

## 5. 真实 PostgreSQL 证据

真实数据库测试从迁移后的隔离 PostgreSQL 执行完整往返：

1. 房主创建后能从权威列表看到房间，未邀请 Principal 看不到。
2. 被邀请 Principal 只看到 `invited` 状态，接受后成为 `joined`。
3. 被移除 Principal 立即从列表消失。
4. 权限、房主和归档状态跨新的仓储进程实例恢复，证明不依赖房主或 Control Plane 的进程内内存。
5. 乐观并发版本冲突明确失败，不允许后写覆盖先写。

`tools/database.py test` 会自行创建、迁移并销毁隔离测试库；私人房间真实测试 2 个通过，数据库全套集成测试全部通过。

## 6. 真实 Synapse 证据

真实 Synapse 验收覆盖了服务端硬边界：

1. Appservice 创建隐藏、邀请制、受管理的私人房间。
2. 相同稳定别名重复创建会解析回同一个 `room_id`。
3. 未受邀用户不能加入；受邀房主和成员可以加入。
4. 默认成员不能发言；授予发言后可以发送，撤销后立即被拒绝。
5. `kick` 后不能发送，`ban` 后不能重新加入。
6. 归档把普通房主降为只读，不能继续发送。

真实依赖验收还暴露并修复了两个只有接上 Synapse 才会出现的协议分类错误：

- 重复别名返回 HTTP 400 `M_ROOM_IN_USE`，现在被映射为可收敛的冲突，而不是通用失败。
- 被封禁用户加入返回 HTTP 403 且正文为 `M_BAD_STATE`，现在 403 优先映射为禁止，不再误判为冲突。

`tools/matrix.py` 的 3 个真实 Synapse 用例全部通过；Matrix Adapter 18 个单测和 Provisioning Adapter 13 个单测全部通过。

## 7. 被移除成员的读取边界

“不能读取后续受保护内容”由三层证据共同成立：

- 领域与 PostgreSQL：移除后不再具备 `View`，也不再出现在权威房间列表。
- 内容授权用例：只有产品 `View` 能力与可信 Matrix 当前成员状态同时成立才允许读取；产品移除事实会覆盖尚未收敛的 Matrix 成员投影。
- Synapse：`kick` / `ban` 先于产品事实提交，立即关闭服务端房间访问和发送边界。

这保证当前服务端模型下的后续访问拒绝。历史明文在已被客户端接收后无法撤回；未来密文的密码学隔离、设备密钥恢复和成员移除后的密钥轮换属于任务 27。

## 8. 质量门禁

```text
just check
  Rust fmt、Clippy -D warnings、workspace 全目标/全特性检查与测试通过
  Application 单测 44/44，私人房间应用流 7/7
  Matrix Adapter 18/18，Provisioning Adapter 13/13，Domain 53/53
  Vitest 37 个文件、134 个测试通过
  TypeScript strict、ESLint、生产构建、协议生成/一致性通过
  484 个文本文件 Secret 扫描、11 个 Actions 固定引用通过

just web-browser
  14 个真实 Chrome 用例通过
  覆盖私人房间桌面创建、安全失败路径、移动端无横向溢出
  公共大厅、连接舱、Composer、消息查看、交接与图形降级回归全部通过

.venv\Scripts\python.exe tools\database.py test
  真实 PostgreSQL 全套测试通过，隔离数据库自动创建、迁移和销毁

.venv\Scripts\python.exe tools\matrix.py
  真实 Synapse 3/3 通过
```

浏览器证据截图保存在 `artifacts/browser/task-25/private-room-security-truth.png`。构建仅保留既有 Matrix Crypto WASM 与 Pixi 大分块提示，没有新增编译、Lint 或测试失败。

## 9. 明确边界与下一步

- 当前邀请目标使用严格 Principal ID，不在本任务偷偷加入不完整的全局用户搜索；目录搜索与隐私策略应作为独立切片设计。
- Matrix E2EE、设备验证、密钥备份和移除后密钥轮换属于任务 27。
- 直接会话复用、屏蔽和精确在线状态保护属于任务 26，也是下一任务。
- 当前真实宿主与浏览器证据以 Windows x64 为主；其他桌面平台仍需各自发行验收。
