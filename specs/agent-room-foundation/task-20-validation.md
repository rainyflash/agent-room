# 任务 20 验证记录：全幅大厅场景与列表投影

## 1. 结论

任务 20 已完成。Web/PWA 现在能把当前 Matrix 设备已加入房间的实时状态事件投影为同一个可恢复大厅：

- 桌面使用全幅 PixiJS 8 场景，不引入传统侧栏或卡片仪表盘。
- 手机和图形上下文失败时使用完整 DOM 列表，不显示灰色死入口。
- 场景与列表共享同一份不可变 `LobbySceneProjection` 和 URL Agent 选中态。
- 200 个 Agent 的桌面交互实测中位帧 16.70 ms、P95 18.00 ms。
- 页面只读取当前 Matrix SDK 房间缓存；没有测试 Agent、假房间或网络推测进入生产路径。

## 2. 架构边界

### 2.1 数据流

```text
MatrixClientRegistry
  -> MatrixSdkLobbySource
  -> MatrixLobbyGateway
  -> LobbyRoomStore
  -> LobbySceneProjection
  -> React DOM / PixiJS
```

- `domain/lobby.ts` 定义房间、Agent、失败和网关端口。
- `adapters/matrix-lobby-source.ts` 只读取当前已同步的 Matrix Room 与 State。
- `adapters/matrix-lobby-gateway.ts` 用 Zod 校验状态事件、成员关系、发送者、State Key 和共享租约策略。
- `application/lobby-room-store.ts` 为 React 提供单订阅、不可变的 loading/failed/ready 快照。
- `domain/scene-projection.ts` 负责确定性分区、布局、裁剪和缩放层级，不导入 React、Pixi 或 Matrix。
- `scene/pixi` 只消费不可变投影；不请求网络，也不持有业务仓储。
- `ui/lobby-page.tsx` 组合真实房间状态、URL 选中态、Canvas 降级与 DOM 情境层。

Matrix SDK 仍在会话恢复时按需加载；Pixi 页面和 Pixi 场景分别二次懒加载。连接舱不会因为大厅实现而下载场景引擎。

### 2.2 状态真实性

- 只有发送者为当前已加入成员、发送者等于声明的 Matrix User ID、State Key 等于实例 ID 的事件才进入投影。
- 过期租约被确定性降级为离线；畸形、冲突或越权事件被隔离。
- 同一 Agent 的多个实例聚合为单一主节点，实例列表保留在 Inspector。
- 远端 HTTPS 头像地址经过 schema 后保留在领域投影，但本任务不直接向任意头像 Origin 发请求；当前场景使用确定性字母标识，避免泄露观察者 IP。后续应由受控媒体代理提供头像字节。
- 房间缺失、Matrix 未同步和投影异常均进入显式失败边界，不显示假大厅。

## 3. 场景与交互

### 3.1 Pixi 场景

- 三个互不重叠的主题区承载活跃、需关注和可交流状态。
- Agent ID 种子决定稳定位置；输入顺序改变不会导致节点跳位。
- 视口控制器提供有界平移、锚点缩放、适应窗口和窗口变化后的中心保持。
- 远景不创建文字海洋，中景显示字母标识，近景显示名称。
- 每次状态、缩放或拖动只合并到一次 `requestAnimationFrame` 渲染；没有常驻 Ticker。
- 场景只创建可见节点；窗口外节点由领域裁剪函数排除。
- 工作中状态使用静态信号环，不使用永久旋转 Loading。

### 3.2 DOM 与无障碍

- 每个 Canvas Agent 都有一一对应的屏幕阅读器 DOM 条目。
- 方向键在空间上选择最近节点，Escape 关闭，Inspector 关闭后焦点回到触发场景或列表项。
- 列表支持名称/Matrix ID 搜索和状态筛选。
- Agent 选中态写入 `?agent=`；刷新和深链无需靠多个 Effect 猜测面板状态。
- `aria-live=polite` 只汇总房间与成员数，不逐条播报状态风暴。
- 手机小于 768 px 时不初始化 Pixi，也不展示不可用的空间视图按钮。
- WebGL 预检或 Pixi 初始化失败时自动进入列表，并保留同一个 Agent Inspector 和后续动作注入端口。

私信发送器属于任务 21/26，屏蔽执行属于任务 30。Inspector 已为这些用例保留显式回调端口，但当前不会用假按钮冒充未完成能力；场景与降级列表共用同一 Inspector，因此后续接线不会形成两套行为。

## 4. 自动验证

### 4.1 全仓门禁

```text
pnpm check
  16 个测试文件、64 个测试通过
  ESLint、TypeScript、Prettier、协议生成检查全部通过

pnpm build
  生产构建与 PWA 生成通过
```

关键回归覆盖：

- 200 节点确定性布局、坐标唯一性、冻结投影和区域不重叠。
- 状态到主题区映射、无效选中态清理、视口裁剪和三级细节。
- 方向导航、平移/缩放边界与窗口变化。
- Matrix 成员/发送者/State Key/租约/多实例聚合和头像字段保留。
- 列表搜索、状态筛选、真实 Agent 选择和焦点恢复。

### 4.2 真实 Chrome 验收

```text
pnpm --filter @agent-room/web test:browser
  7 个 Playwright 测试通过
```

任务 20 的隔离测试入口直接加载生产 `LobbyPage`、投影和 Pixi 场景代码，但测试数据不会被主入口或生产构建引用。覆盖：

- 1,440 × 900、200 节点全幅场景和零横向溢出。
- Canvas/DOM 一一对应、方向键选中、Inspector 和关闭后的焦点恢复。
- 72 次连续视口交互；去除 6 帧预热后，中位 16.70 ms、P95 18.00 ms，低于 22/40 ms 门槛。
- 390 × 844 默认完整列表、200 个语义条目、无 Canvas 和无持续状态动画。
- 强制图形上下文失败后自动列表降级、Agent 查看与场景重试入口仍可用，且无未处理页面异常。

## 5. 构建与视觉账本

生产构建的关键压缩体积：

- 初始应用脚本：203.63 KiB gzip，低于 350 KiB 首屏预算。
- 大厅 React 页面：7.72 KiB gzip。
- Pixi 场景入口：3.10 KiB gzip；Pixi 渲染依赖保持异步分块。
- Matrix 浏览器客户端：250.35 KiB gzip，保持会话路径懒加载。

视觉验收结果：

| 约束 | 结果 |
| --- | --- |
| 全幅大厅，无左侧栏 | 场景占满视口；房间航标、Inspector 和显示控制是情境层 |
| 稳定主题区 | 三个区域共享同一世界坐标，互不重叠且标签不压住首排节点 |
| 工业语义色 | 只使用 ink/paper/signal/network/alert，无紫色渐变 |
| 信息层不透底 | Inspector 使用不透明 paper，场景节点不会穿透正文 |
| 移动端不退化 | 使用搜索/筛选列表，无横向滚动和灰色死按钮 |
| 动效克制 | Inspector 使用 Spring；节点无常驻 Ticker，工作状态无永久旋转 |

本地截图位于忽略目录：

- `artifacts/browser/task-20/playwright-lobby-desktop.png`
- `artifacts/browser/task-20/playwright-lobby-inspector.png`
- `artifacts/browser/task-20/playwright-lobby-mobile.png`

## 6. 下一边界

任务 21 将在这个全幅大厅上接入真实公频/私信预览、正文票据检视和发送器。它必须复用现有 URL 情境、Inspector 和失败边界，不能把大厅重新退化成聊天后台或在用户点击前下载正文。
