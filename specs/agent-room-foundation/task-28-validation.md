# 任务 28 验证记录：多设备同步、恢复与设备管理

## 1. 结论

任务 28 已完成。实现没有把所有“设备”粗暴塞进一张表，而是保留三条不同的权威边界：

- **Matrix 密码学设备**：由 Matrix Crypto、交叉签名、SAS、Secret Storage 和密钥备份管理。
- **Agent Room 产品设备**：由 OIDC 主体、设备持有证明、Token Family 和产品撤销状态管理。
- **Agent 实例**：每个运行中的 Agent 进程拥有独立实例标识、租约、签名公钥和 Matrix Device。

安全中心分别展示这三类事实。产品设备撤销会立即关闭本地 Token 与其上全部 Agent 实例；单实例撤销只停止目标实例。Matrix 会话清理是撤销后的独立、可重试副作用，失败不会把已经完成的本地撤销回滚成“仍可访问”。

语言与大厅视图使用 Matrix Account Data 跨设备同步。字段级收敛寄存器、确定性冲突顺序和读—合并—写—确认循环避免整份文档的最后写覆盖。两个独立客户端在相反服务端写入顺序下均已证明最终收敛。

房间、历史、已读位置和 E2EE 恢复继续使用 Matrix 的唯一事实源，不另建一套产品同步数据库。上下文交付沿用一次性交接协议，并要求用户选择具体 `AgentInstanceId`。

## 2. 三类设备事实不可混淆

| 对象 | 权威源 | 撤销影响 | 不负责 |
| --- | --- | --- | --- |
| Matrix 密码学设备 | Matrix + Crypto Store | 后续加密收件、设备信任与历史密钥恢复 | 产品 Token、Agent 租约 |
| 产品设备 | PostgreSQL 设备、Token Family、发送方持有证明 | Access/Refresh Token、该设备全部 Agent 实例 | 人类 Matrix 交叉签名状态 |
| Agent 实例 | PostgreSQL Agent Instance + 独立 Matrix Device | 目标实例租约、签名资格和专属 Matrix 会话 | 同一主机上的其他实例 |

安全中心把 Matrix 信任区与“授权访问”账本分开。授权访问区再把产品设备和 Agent 实例分栏，展示平台、最近活动、产品信任状态、实例在线状态和 Matrix 清理状态。UI 不根据心跳自行猜测授权，所有列表均读取控制平面权威结果并经过严格 Zod 解码。

## 3. 账户偏好同步与冲突规则

账户偏好保存在 Matrix Account Data 事件 `org.agentroom.preferences.v1` 中，当前同步字段为：

- `language`：`system | en | zh-CN`
- `lobbyView`：`scene | list`

文档使用严格 `schemaVersion: 1`，每个字段是独立寄存器：

```text
{ logicalClock, writerId, value }
```

`writerId` 使用 Matrix Device ID。合并顺序依次比较逻辑时钟、写入设备和字段值，因此任意两个有效文档都能得到确定结果；不同字段的并发修改不会互相覆盖。同字段并发修改也有稳定胜者，不依赖事件到达顺序。

同步仓库执行以下闭环：

1. 读取远端文档。
2. 按字段合并本地待确认文档与远端事实。
3. 只有候选文档不同于远端时才写入。
4. 写入后再次读取确认；发现竞争写入则继续合并和对账。
5. 断线写入保持 `pending`，Matrix 客户端恢复活动后自动重试。
6. 登录账户在异步读取期间变化时，旧账户响应被丢弃，不能污染新账户。

两个独立 `AccountPreferencesStore` 共享一个会制造整份 Account Data 覆盖的竞争服务器夹具。测试分别让设备甲和设备乙最后写入；两种情况下服务端和两端最终都收敛到 `{ language: zh-CN, lobbyView: list }`。

## 4. 产品设备撤销

产品设备撤销在单个 PostgreSQL 事务中完成：

- 把设备转为 `revoked`。
- 撤销全部活跃 Token Family、Access Token 和 Refresh Token。
- 把该设备上的 Agent 实例转为 `revoked`、清空租约并记录撤销时间。
- 写入 `device.revoked.v1` Outbox 安全事件。
- 返回仍需清理的 Agent Matrix Device 列表。

事务提交后才调用 Matrix。这样 Matrix 不可用时，本地授权边界仍已关闭，旧 Token 立即失败。响应会明确返回 `matrixCleanup: pending` 和剩余实例数；重复同一撤销请求会继续清理未完成目标，直到收敛，而不是重复制造撤销事件。

## 5. Agent 实例撤销与 Matrix 清理

`GET /agent-instances` 只列出当前主体拥有或可操作的实例，记录包含 Agent、宿主设备、Adapter、在线状态、最近活动、Matrix Device 和清理时间。

`DELETE /agent-instances/{instance_id}` 要求活跃 Web 会话、精确同源和近期认证。应用服务只依赖实例仓储、撤销事务、Matrix 清理端口、时钟和标识工厂；HTTP 和 SDK 细节不会渗入用例层。

实例撤销先原子完成：

- `status = revoked`
- `lease_expires_at = NULL`
- 写入撤销时间和 `agent.instance.revoked.v1` Outbox 事件

随后 Matrix Provisioning Adapter 只对受管 Agent 用户调用设备删除接口，并安全编码 User/Device 路径。设备已经不存在等价于清理成功；普通人类 Matrix 用户在发请求前即被拒绝。网络、鉴权或状态持久化失败映射为稳定的待清理原因，UI 提供安全重试。

迁移 `202608250002_agent_instance_management.sql` 增加 `matrix_device_revoked_at`、撤销顺序约束和待清理部分索引。数据库不允许出现“Matrix 已清理但本地实例尚未撤销”的矛盾状态。

## 6. 精确目标实例交付

交接协议的 `targetInstanceId` 是必填 UUIDv7，不接受只选择 Agent 或主机。Web 在提交前加载当前主体有权访问的活跃实例，以选择框展示完整实例集合；批准请求、Matrix To-Device 事件、Bridge 接收校验、一次性上下文存储和终态回执始终携带同一目标实例。

浏览器测试确认“打开正文”和“交给 Agent”仍是两个独立动作，用户必须在确认面板选择 `Exact target instance`。真实 PostgreSQL 测试建立两个实例并验证交接授权查询返回精确的请求端和目标端；目标撤销后 `active = false`，不能继续接收新交付。

## 7. 房间、历史、已读与全部设备丢失

房间、时间线和已读位置继续由 Matrix 同步；Web 与 Bridge 不把 PostgreSQL 投影冒充聊天历史。E2EE 历史恢复沿用任务 27 的 Secret Storage 和密钥备份：新设备即使没有任何旧本地 Crypto Store，也能在持有外部恢复口令或恢复密钥时导入备份并解密同一条真实 Megolm 事件。

这里存在不可绕过的密码学边界：

- 丢失全部本地设备，但仍持有外部恢复凭据：可以恢复。
- 全部设备和恢复凭据同时丢失：旧 E2EE 历史不可恢复，只能建立新的加密身份。

第二种情况不是产品缺陷，而是端到端加密不保留服务端万能后门的直接结果。UI 不会用重新登录、产品管理员权限或本地估算伪装历史恢复成功。

## 8. API 与用户体验

新增或完成的管理接口：

| 方法与路径 | 行为 |
| --- | --- |
| `GET /auth/devices` | 列出当前主体的产品设备 |
| `DELETE /auth/devices/{device_id}` | 级联撤销产品设备、Token 和实例；报告 Matrix 待清理数 |
| `GET /agent-instances` | 列出当前主体可管理的 Agent 实例 |
| `DELETE /agent-instances/{instance_id}` | 撤销单一实例并清理专属 Matrix Device |

控制平面响应统一 `Cache-Control: no-store`。Web 客户端对成功与失败载荷都做严格 Schema 校验，未知响应不会被当作成功。TanStack Query 负责服务端状态；撤销成功后失效设备与实例查询，不用 `useEffect` 手工复制缓存。

撤销交互包含影响摘要、取消入口、Spring 展开动画、进行中禁用、失败边界和 Matrix 待清理提示。英文与简体中文资源完整；桌面双栏、390px 单栏均无横向溢出。

## 9. 验证证据

### TypeScript 与浏览器

```text
pnpm check
  Prettier、ESLint、TypeScript、协议一致性通过
  52 个 Vitest 文件、193 个测试通过

pnpm build
  Protocol 包与 Web 生产构建通过

pnpm --filter @agent-room/web test -- --run \
  src/features/preferences/application/account-preferences-store.spec.ts
  7/7 通过，含两个相反覆盖顺序的双设备收敛

pnpm --filter @agent-room/web exec playwright test e2e/security-center.e2e.ts
  2/2 通过
  桌面访问账本、撤销影响确认、SAS、390px 布局与恢复入口通过
```

浏览器证据：

- `artifacts/browser/task-28/security-desktop.png`
- `artifacts/browser/task-28/access-mobile.png`
- `artifacts/browser/task-28/security-mobile.png`

### Rust、PostgreSQL 与真实 Matrix

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
  全部通过

python tools/database.py test
  真实 PostgreSQL 全部通过
  含设备级联撤销、旧 Token 拒绝、实例撤销、重复 Matrix 清理收敛和精确交接实例

python tools/vertical.py security
  真实 Synapse 纵向场景 1/1 通过
  三个独立浏览器设备完成交叉签名、SAS、正数密钥备份恢复和原事件解密
```

Matrix 临时不可用时，纵向测试会出现预期的同步或缺少房间密钥警告；恢复后必须解密原事件才算通过。Windows MSVC 的本地化链接器提示和第三方 `proc-macro-error2` 未来兼容提示不是门禁失败。

## 10. 变更提交

- `67e7ebc`：定义可收敛账户偏好领域模型。
- `b8e6bb8`：接入 Matrix Account Data 同步仓库。
- `4792e5e`：应用跨设备语言与大厅视图偏好。
- `ee93ea4`：定义失败关闭的实例撤销用例。
- `33fec0a`：持久化实例撤销和 Matrix 清理状态。
- `3197528`：实现受管 Agent Matrix Device 撤销。
- `362c246`：公开 Agent 实例管理 API。
- `21efbff`：产品设备撤销级联至 Agent Matrix 会话。
- `b54c46e`：增加严格访问管理客户端。
- `b334424`：完成产品设备与 Agent 实例管理 UI。
- `38cc0a7`：把偏好 Provider 测试固定到真实 DOM 环境。
- `a76b52b`：证明双设备竞争最终收敛。
- `b7dfa74`：固化访问管理浏览器验收。
- `e15937f`：让收敛夹具通过全仓严格类型风格门禁。

## 11. 下一步

下一项是任务 29：自动发言授权。必须按房间、Agent、实例、消息类别、受众、频率、总量和期限建模，并默认关闭。设备或实例“在线”绝不等价于获得自动发言权。
