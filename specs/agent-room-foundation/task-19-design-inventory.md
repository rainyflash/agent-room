# 任务 19：Web 壳设计库存与会话边界

> 状态：已确认，作为任务 19 的实现与验收账本  
> 依赖：[界面与交互设计规范](./ui-design.md)、[安全与隐私设计](./security.md)、[总体技术设计](./design.md)

## 1. 本任务交付边界

任务 19 只交付真实的应用壳、认证、Matrix 增量同步、故障恢复、路由和国际化。它不伪造大厅节点、房间成员或聊天时间线；大厅场景、消息检视器和上下文交付分别属于任务 20、21 和 22。

连接舱必须始终回答三个问题：

1. 当前处于哪一个确定阶段。
2. 阻塞来自浏览器、控制平面、身份提供方还是 Matrix。
3. 用户现在能执行哪个恢复动作。

不得使用没有解释的灰色主按钮，也不得把依赖失败吞成永远旋转的加载态。

## 2. 已确认视觉方向

- **风格**：工业/实用主义，像可信的通信控制台，不像通用 SaaS 卡片后台。
- **桌面结构**：左侧深色生命周期轨道约占 30%，右侧浅色操作面铺满剩余窗口；没有固定导航侧栏。
- **移动结构**：顶部身份与语言入口，生命周期阶段纵向展开，当前故障与恢复动作紧邻展示。
- **色彩**：`ink #111310`、`paper #F2F0E9`、`signal #9FE870`、`network #66C9D8`、`alert #FF6B3D`。
- **字体**：Instrument Sans；中文使用与 Source Han Sans SC 同源字形的本地 CJK Web Font；技术字段使用 IBM Plex Mono。
- **图标**：仅使用 Lucide 风格线性图标，不使用 Emoji。
- **动效**：只在阶段变化和故障恢复时使用 Motion Spring；`prefers-reduced-motion` 下关闭位移动效。

本地生成的实现参考图位于被忽略的验收产物目录：

- `artifacts/design/task-19-connection-desktop.png`
- `artifacts/design/task-19-connection-mobile.png`

图像只约束信息层级、比例和视觉语义，不是可以照抄虚构数据的产品截图。

## 3. 组件库存

| 组件 | 单一职责 | 数据来源 |
| --- | --- | --- |
| `ConnectionRail` | 展示启动阶段和当前阶段 | XState snapshot |
| `ConnectionWorkspace` | 展示阶段说明、故障和恢复动作 | XState snapshot |
| `DependencyHealthStrip` | 展示控制平面、Matrix 和网络状态 | TanStack Query + Matrix 连接事件 |
| `IdentitySummary` | 展示已认证主体，不推导身份 | `GET /auth/session` |
| `LanguageControl` | 系统语言与显式覆盖 | i18next + localStorage 偏好 |
| `ShellHeader` | 产品标识、环境和辅助动作 | 路由 + 配置 |
| `RouteUnavailable` | 明示后续任务尚未交付 | 路由，不使用假数据 |

React 组件只渲染 snapshot 和发送事件。认证、同步与恢复规则不进入 JSX。

## 4. 会话边界

浏览器有两条独立会话，不能混为一谈：

### 4.1 控制平面会话

- 通过控制平面的 OIDC Authorization Code + PKCE 流程建立。
- 使用控制平面 Origin 上的 `Secure`、`HttpOnly`、`SameSite=Lax` Cookie。
- 前端通过 `GET /auth/session` 获取最小主体投影，不接触 OIDC Token。
- 跨 Origin 请求只允许配置中的精确前端 Origin，并携带凭据。

### 4.2 Matrix Web 设备会话

- 浏览器直接使用 Matrix Client-Server SSO 登录，同一 OIDC Provider 负责稳定主体映射。
- SSO 回调的单次 `loginToken` 立即兑换为 Matrix 设备会话，并从地址栏移除。
- Matrix Access Token 只保存在当前 `sessionStorage` 生命周期中，用于刷新恢复；关闭浏览器会话后重新走 SSO。
- Matrix 同步缓存使用官方 SDK 的 IndexedDB Store，可丢失、可重建。
- 返回的 Matrix User ID 必须与控制平面 `matrixUserId` 完全一致；不一致时注销新会话并停止同步。
- Application Service Token 只服务 `_agent_<uuid>` 独占命名空间，永远不进入 Web 构建、响应或存储。

任务 27 接入 E2EE 时，再由 Matrix SDK 的加密 Store 管理设备密钥与恢复。任务 19 不自写密码学，也不虚构加密已就绪。

## 5. URL 状态契约

顶层路径：

```text
/connect
/lobby/:catalogId
/lobby/:catalogId/instance/:roomId
/settings/:section
/admin/:scope
```

大厅情境查询：

```text
?agent=<agentId>&message=<messageId>&directory=open
```

查询参数由 schema 校验。无效可选参数被丢弃并规范化；无效必需路径进入明确的不可用页面。Matrix SSO 的 `loginToken` 只允许出现在 `/connect`，兑换后使用 replace navigation 清除。

## 6. 生命周期状态机

```text
booting
  -> unauthenticated(control)
  -> restoring

unauthenticated(control) -> authenticating(control) -> restoring
unauthenticated(matrix)  -> authenticating(matrix)  -> restoring

restoring -> syncing -> ready
restoring/syncing -> degraded | offline
ready -> reconnecting -> ready | offline
degraded -> ready | reconnecting | offline
offline -> reconnecting
```

- XState 管理生命周期和恢复动作。
- TanStack Query 管理控制平面健康与服务端查询缓存。
- 浏览器 `online/offline` 事件只是信号，不被当作服务真实可达性的证明。
- 每个失败态都保留 `correlationId`、失败边界和可执行的重试/重新登录动作。

## 7. 响应式与无障碍

- `>= 900px`：左右分区；轨道固定在视口内，操作面独立滚动。
- `< 900px`：单列；轨道变成可扫描阶段列表，主操作位于当前状态之后。
- 所有按钮保持至少 44 px 触控目标，键盘焦点使用 2 px `signal` 环。
- 阶段变化通过 `aria-live=polite` 汇总，错误正文不逐字符播报。
- 状态由文字、图形和颜色共同表达。
- 所有字符串允许至少 35% 膨胀，不用固定宽度截断关键错误。

## 8. 视觉保真账本

浏览器验收必须逐项对照：

1. 桌面左右面积比例是否接近 30/70，且无传统侧栏和居中卡片。
2. 当前阶段是否同时具备编号、状态符号、标题和辅助说明。
3. 故障是否在右侧工作面形成清晰的主层级，并紧邻恢复动作。
4. 健康条是否只显示真实依赖状态、延迟和最后检查时间。
5. `ink/paper/signal/network/alert` 是否承担固定语义，没有紫色渐变或玻璃卡片。
6. 390 px 宽度下是否无横向滚动、无被裁切主操作、无缩成邮票的桌面布局。
7. reduced-motion、键盘顺序、深链刷新和离线恢复是否可用。
8. 控制台是否无未处理异常、重复请求风暴和 Matrix Token 泄漏。

