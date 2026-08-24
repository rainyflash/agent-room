# 任务 22 验证记录：一次性上下文交付闭环

## 1. 结论

任务 22 的协议、领域状态机、应用用例、Matrix 加密传输、本地加密存储和显式授权 UI 已完成：

- “打开完整正文”和“交给 Agent”是两个独立动作；前一个动作不会隐式触发后一个动作。
- 用户必须再次确认精确目标实例、内容范围、用途和过期时间，未确认时不会调用交付端口。
- 交付请求和终态回执均由实例身份签名，并只通过 Matrix 加密 To-Device 通道传输。
- 接收 Bridge 在下载正文前校验目标、授权、实例签名、时间窗和重放状态，下载后再次核对内容摘要。
- 正文只以 XChaCha20-Poly1305 密文写入本地一次性存储；消费、拒绝、撤销或过期后不能再次读取正文。
- 发送方用终态回执收敛已消费、已拒绝、已撤销、已过期和失败状态；回执重复到达不会产生第二次副作用。

核心提交：

- `807b08c`：签名交付协议与生成类型。
- `a62fe3a`：一次性交付领域生命周期。
- `0f17659`：发送侧批准、投递和对账用例。
- `fad0b65`：接收侧验证、消费和拒绝用例。
- `b5e450f`：终态回执及幂等收敛。
- `fc21a58`：Matrix 加密 To-Device 适配器。
- `d86b5c5`：本地加密一次性上下文存储。
- `7f628ce`：Bridge 启动期安全存储迁移与密钥装载。
- `904a4fe`：Web 显式交付意图模型。
- `a5328b2`：用户确认、进度、撤销和失败 UI。

## 2. 架构与依赖方向

```text
HandoffPanel
  -> HandoffDeliveryMachine
  -> HandoffGateway
     -> WebObserverHandoffGateway（生产 Web 安全拒绝）
     -> Bridge 私有 IPC（任务 23 装配）

HandoffDeliveryService
  -> AgentDirectory + Authorization + Signer + HandoffStore
  -> MatrixSdkHandoffGateway
  -> 加密 To-Device 请求

MatrixSdkHandoffGateway
  -> HandoffReceptionService
  -> ContentReader + SignatureVerifier + HandoffStore
  -> 加密一次性上下文包 + 终态回执
```

- `domain` 只定义交付实体、合法状态转换和稳定标识，不依赖 Matrix、SQLite、React 或 Codex。
- `bridge-core` 编排授权、签名、摘要、时效、消费和回执，不直接操作数据库或 SDK。
- `matrix-adapter` 只负责精确设备寻址、加密 To-Device 收发和 SDK 错误映射。
- `bridge-storage-adapter` 只实现本地事务、XChaCha20-Poly1305 密文和一次性删除语义。
- Web 组件只呈现状态机；它不会持有 Agent 私钥，也不会自行创建 Matrix 会话。

## 3. 显式授权与最小权限

正文检视器只有在完整正文通过票据、长度、媒体类型和 SHA-256 校验后，才显示“交给 Agent”入口。打开交付面板后仍需完成第二次确认：

| 字段 | 约束 |
| --- | --- |
| 目标 | 必须选择目录返回的精确 Agent 实例，不能只选 Agent 或设备别名 |
| 范围 | `full_content` 或 `selected_excerpt`，选择节选时必须提交明确范围 |
| 用途 | 必须是有界、非空的人类可读说明 |
| 有效期 | 当前 UI 只允许 5、15 或 60 分钟 |
| 权限 | 只允许一次性读取上下文；不会顺带授予工具调用、自动发送或工作区访问 |

状态机在确认前不调用 `approve`。目标加载失败、批准失败、提交结果未知、回执查询失败和撤销失败都使用稳定错误码显示恢复动作；提交未知时只按原 Handoff ID 对账，不能生成新交付盲目重试。

## 4. 加密传输与接收验证

发送侧按以下顺序执行：

1. 根据用户确认构造不可变交付意图。
2. 查询 Agent 目录并锁定目标实例及其 Matrix Device。
3. 校验当前主体对源内容和目标 Agent 的授权。
4. 使用源实例 Ed25519 密钥签名规范化请求。
5. 刷新目标设备身份，并用 Matrix SDK 的 Olm 加密会话发送 To-Device 事件。
6. 持久化提交状态；网络结果不确定时进入对账而不是重发。

接收侧只接受 SDK 标记为 `OlmV1Curve25519AesSha2` 解密结果的事件。明文、自报已加密或缺少发送者设备身份的事件全部拒绝。进入内容服务前依次验证：

- 收件 Agent 实例和 Matrix Device 是否精确匹配当前 Bridge。
- 发送实例是否仍属于声明 Agent，且授权没有撤销。
- 签名是否覆盖 Handoff ID、源和目标、内容引用、范围、用途及时间窗。
- 请求是否尚未过期、是否已经处理过。
- 下载后的正文长度、媒体类型和摘要是否与已签名引用完全一致。

任一步失败都不会创建本地上下文包，也不会把正文暴露给宿主。

## 5. 一次性存储与终态收敛

- 本地存储使用每台 Bridge 的 OS 安全存储密钥；SQLite 只保存 XChaCha20-Poly1305 密文、随机 nonce 和认证所需元数据。
- Handoff 元数据作为 AEAD 附加认证数据，数据库密文、nonce、元数据或密钥任一被篡改都会解密失败。
- `consume` 在同一事务中校验目标实例和时效、解密正文、把状态改为 `consumed` 并删除持久正文；第二次消费失败。
- `decline`、`revoke` 和 `expire` 同样删除持久正文并生成签名终态回执。
- 重复请求返回既有交付结果，不重复下载或写入；重复回执保持同一终态。
- 终态不会从 `consumed`、`declined`、`revoked` 或 `expired` 回退到 `delivered`。

## 6. 安全验收矩阵

| 场景 | 预期结果 | 自动证据 |
| --- | --- | --- |
| 未点击确认 | 不调用交付网关 | Web 状态机与面板测试 |
| 错目标实例 | 下载前拒绝，不创建上下文包 | 接收流程与存储测试 |
| 请求签名被篡改 | 下载前拒绝 | 接收流程测试 |
| 正文内容或摘要被篡改 | 校验失败，不写入存储 | 接收流程测试 |
| 本地密文、nonce 或密钥错误 | AEAD 解密失败 | 加密存储测试 |
| 重放同一请求 | 返回既有结果，不重复下载 | 接收流程与存储测试 |
| 重复消费 | 第一次成功，后续全部失败 | 接收流程与存储测试 |
| 请求或本地包过期 | 不下载或原子过期并删除正文 | 接收流程与存储测试 |
| Matrix 明文 To-Device | 适配器拒绝，不进入用例 | Matrix 适配器测试 |
| 投递结果未知 | 复用原标识对账，不盲目重发 | 发送流程与 Web 状态机测试 |
| 终态回执重复或顺序异常 | 幂等收敛，不回退终态 | 发送流程与存储测试 |

## 7. 交互与浏览器验收

- 以任务 21 已接受的内容检视器为视觉基线，没有另造第二套设计语言。
- 桌面 1440 × 900 下验证：打开正文、进入交付、选择唯一目标、确认、显示已交付、查询一次性上下文和撤销。
- 移动 390 × 844 下验证：交付表单单列排布、按钮可见可点、无横向溢出。
- 界面明确说明远端正文仍是不可信输入，交付不等于授予工具或发送权限。
- 生产 Web 使用 `WebObserverHandoffGateway` 并稳定返回 `handoff.bridge_unavailable`；浏览器不能绕过私有 IPC 直接访问 Bridge。

本地截图位于忽略目录：

- `artifacts/browser/task-22/iab-handoff-form-desktop.png`
- `artifacts/browser/task-22/iab-handoff-delivered-desktop.png`
- `artifacts/browser/task-22/iab-handoff-form-mobile.png`

## 8. 质量门禁

```text
pnpm check
  34 个测试文件、127 个测试通过
  Prettier、ESLint、TypeScript、协议生成检查全部通过

pnpm build
  4 个工作区项目构建通过
  Web 生产构建与 PWA 生成通过

pnpm --filter @agent-room/web exec playwright test e2e/handoff-delivery.e2e.ts
  Chromium 桌面与移动端 2 个用例通过

just check
  Rust fmt、Clippy -D warnings、workspace 全特性测试通过
  TypeScript 检查、生产构建、协议一致性、Secret 扫描和 Actions 固定引用检查通过
```

关键 Rust 回归覆盖 6 个发送流程、6 个接收流程和 9 个加密存储场景；Matrix 适配器同时覆盖 Olm 密文接收和明文拒绝。

## 9. 明确边界

任务 22 完成的是可以由真实运行时装配的完整交付能力，不代表浏览器演示夹具已经成为生产链路：

- 当前 Bridge 启动时会创建、迁移并验证加密 Handoff Store，但常驻 Agent Matrix Device 同步循环要在任务 24 的纵向装配中启动。
- 生产 Web 页面没有本地私有 IPC 权限，因此故意安全失败；它不会开放 localhost HTTP 后门。
- Codex 的 `consume_handoff`、`decline_handoff` 以及其他工具通过 Bridge 私有 IPC 暴露，属于任务 23。
- 真实 Codex 中完成“登录 → Agent 上线 → Matrix 收件 → 一次消费 → 回复”的端到端证据属于任务 24。

这个边界不是功能降级，而是依赖方向：交付领域和适配器先完成，插件只做薄客户端，最终组合根再连接全部真实运行时。任何层都不得为了提前展示成功状态而复制 Matrix 会话、读取 Codex 私有缓存或绕过 Bridge。
