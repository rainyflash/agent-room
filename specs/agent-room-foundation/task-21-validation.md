# 任务 21 验证记录：信号坞、正文检视器与发送器

## 1. 结论

任务 21 的 Web 交互与应用边界已完成：

- 大厅底部提供默认单行的统一信号坞，展开后可过滤、冻结和恢复观察快照。
- 信号模型覆盖公频、私信、提及、任务引用、待交付和同步异常六类事实；UI 不用条件链拼装类型行为。
- 选择预览不会读取正文；只有用户明确点击后才会申请短期票据、下载字节并校验长度、媒体类型和 SHA-256。
- 受限 Markdown 不执行 HTML，不激活链接；附件不会自动打开或执行。
- 发送器展示身份来源、目标房间、敏感级别、风险标签以及上传、提交、未知、对账、失败和成功状态。
- 发送状态未知时只允许按原提交标识对账，不允许生成新提交后盲目重发。
- Web/PWA 不持有 Agent 实例私钥，因此生产 Web 适配器会明确拒绝伪造 Agent 签名；真实签名发送留给任务 31 的桌面 Bridge 装配。

## 2. 架构边界

### 2.1 入站消息与信号

```text
MatrixMessageSource
  -> MatrixMessageGateway
  -> MessageRoomStore
  -> MessageSignalProjector
  -> SignalDock
  -> ContentInspector
  -> ContentGateway + ContentVerifier
```

- `domain/message.ts` 定义消息、预览、正文引用、生命周期和签名可信度，不依赖 React、Matrix 或 HTTP。
- `adapters/matrix-message-gateway.ts` 只投影当前 Matrix 缓存中的预览事件；它校验房间、发送者、媒体类型、标识和载荷边界，不请求正文。
- `application/message-room-store.ts` 把读取和订阅收敛为不可变快照。
- `domain/signal.ts` 是公频、私信、提及、任务、交付和同步异常的统一展示协议。
- `adapters/message-signal-projector.ts` 只把已有消息事实转成信号，不补造正文、身份或风险。
- `ui/signal-dock.tsx` 使用类型到展示策略的映射；新增信号类型必须先扩展领域注册表，不能在组件里继续堆 `if-else`。
- `ui/content-inspector.tsx` 只在显式动作后调用正文端口，下载结果通过校验前不会进入渲染器。

生产 Web 路径当前只接入真实公频消息投影。私信、提及、任务引用和待交付的统一信号接口及展示策略已经存在，但它们的真实生产来源分别属于任务 25、26 和 22；测试夹具不会进入主入口或生产构建。

### 2.2 出站消息

```text
MessageComposer
  -> MessagePublicationMachine
  -> MessagePublisher
     -> WebObserverMessagePublisher（拒绝签名发送）
     -> Desktop Bridge Publisher（任务 31 装配）
```

- 发送请求使用 UUIDv7 提交标识，状态机持有一次发送意图的唯一真相。
- 上传成功、Matrix 提交未知、正文绑定未知和对账结果分别建模，不用一个布尔值掩盖部分成功。
- `RETRY` 只复用原提交标识；`RECONCILE` 只查询既有提交，不会制造第二条消息。
- 身份只在打开发送器时惰性解析，页面加载不会为了显示按钮而读取 Agent 身份。
- 纯 Web 适配器稳定返回 `publication.bridge_unavailable`，不会拿浏览器用户会话伪装成 Agent 实例签名。

任务 18 已交付真实 Matrix 发送、Ed25519 签名、持久提交状态和对账用例；本任务只把这些边界暴露为可用、诚实的 UI。Bridge 常驻进程和 WebView IPC 的最终生产装配仍属于任务 31，不能把夹具中的成功发送演示谎称为生产联网发送。

## 3. 信任与内容安全

### 3.1 签名可信度

入站事件现在必须携带显式的事件级可信状态：

| 状态 | 含义 |
| --- | --- |
| `instance_verified` | 可信适配器已使用 Agent 实例公钥完成密码学验签 |
| `matrix_sender_matched` | 只确认 Matrix 发送者与载荷声明一致，当前客户端未重验实例签名 |
| `revoked_after_event` | 事件之后实例已撤销，历史事件需显示撤销情境 |

Web Matrix 投影永远只会产生 `matrix_sender_matched`。即使不可信载荷自行声明 `instance_verified`，该字段也会被忽略。内容检视器同时显示房间 ID、来源类型和真实验签层级，不使用假绿色可信状态。未来只有能读取控制平面验签材料并完成 Ed25519 验证的 Bridge 适配器可以产生 `instance_verified`。

### 3.2 正文闸门

- 预览只含标题、摘要、媒体类型、长度、敏感级别、风险标签和不可变正文引用。
- 点击“打开完整正文”前，票据申请次数和正文下载次数都为零。
- 打开后先申请短期票据，再下载不可信字节，最后核对响应长度、媒体类型和 SHA-256。
- 任一阶段失败都保持正文关闭，并显示稳定错误码与可选关联标识。
- Markdown 使用受限语法树；原始 HTML 和 `javascript:` 链接只按文本显示。
- 非文本附件只能在完整性验证后由用户明确下载，不自动预览、执行或交给 Agent。

“打开正文”只代表给人查看，不代表把内容放进 Agent 上下文。后一个权限动作属于任务 22，必须继续保持独立确认和一次性授权。

## 4. 交互与视觉验收

- 信号坞折叠高度不超过 60 px，不遮挡大厅中央控制坞或发送入口。
- 展开后最多渲染 50 条可见信号，支持按实际存在的类型过滤。
- 冻结会保留不可变观察快照；恢复后重新显示最新投影。
- 内容检视器使用不透明纸色表面，桌面宽度 500 px；390 px 视口下占满可用宽度且无横向溢出。
- 签名信任条在桌面双列、移动端单列；房间标识可截断但保留完整 `title`。
- 发送器和信号坞在窄屏使用底部安全区，不与大厅控制层互相覆盖。
- Inspector 与发送器使用 Spring 动画，并尊重 `prefers-reduced-motion`。

## 5. 自动验证

### 5.1 全仓门禁

```text
pnpm check
  31 个测试文件、115 个测试通过
  Prettier、ESLint、TypeScript、协议生成检查全部通过

pnpm build
  4 个工作区项目构建通过
  Web 生产构建与 PWA 生成通过
```

全仓检查还发现并修复了 `ControlPlaneContentClient` 对 `window.setTimeout` 的浏览器硬耦合；现在计时器和联网状态通过 `globalThis` 安全读取，同一适配器可在 Web、Node 测试和未来桌面运行时工作。

关键单元回归覆盖：

- Matrix 房间、发送者、媒体类型、重复事件和越权编辑隔离。
- 不可信事件不能自行升级为实例签名已验证。
- 预览选择零正文网络，显式打开才执行票据、下载和校验。
- HTML 与危险链接保持惰性文本。
- 六类信号策略、风险优先级、稳定排序、过滤和冻结快照。
- UUIDv7 提交标识、发送状态机、未知提交对账和 Web 拒绝伪签名边界。

### 5.2 真实浏览器验收

```text
pnpm --filter @agent-room/web test:browser
  10 个 Playwright 测试通过
```

浏览器回归验证：

- 桌面信号坞默认单行且不遮挡发送器或大厅控制坞。
- 预览被选中后，票据与下载计数仍为零。
- 显式打开后票据和下载各执行一次，校验通过后才显示正文。
- 内容检视器显示真实房间 ID 和 `matrix_sender_matched`，不存在伪造的 `instance_verified` 文案。
- 恶意样例中的 HTML 和 `javascript:` 链接没有生成可执行 DOM 节点。
- 390 px 视口下内容检视器无横向溢出，信任信息改为单列。
- 页面没有未处理异常、控制台错误或资源失败。

本地截图位于忽略目录：

- `artifacts/browser/task-21/playwright-content-inspector.png`

## 6. 构建账本与下一边界

当前生产构建关键压缩体积：

- 大厅页面分块：16.10 KiB gzip。
- 初始应用脚本：211.53 KiB gzip。
- Matrix 浏览器客户端：250.34 KiB gzip，继续保持异步分块。
- Matrix 加密 WASM：2.08 MiB gzip，作为独立资源加载。

下一步是任务 22：把“人类打开正文”和“显式交给某个 Agent 实例”做成两个不可混淆的权限动作，并实现加密 To-Device 交付、一次性上下文包、消费回执、拒绝、撤销与过期。任务 22 不得直接复用正文打开按钮，也不得让已查看正文自动进入宿主上下文。
