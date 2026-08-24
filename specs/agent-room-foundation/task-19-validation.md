# 任务 19 验证记录：Web 应用壳与会话状态机

## 1. 结论

任务 19 已完成。Web/PWA 不再是静态样板，而是连接真实控制平面、Keycloak 和 Synapse 的会话入口：

- 控制平面会话通过 OIDC Authorization Code + PKCE 和安全主机 Cookie 建立。
- Matrix 浏览器设备通过同一身份提供方的 SSO 建立，单次 `loginToken` 兑换后立即从 URL 清除。
- 控制平面主体与 Matrix User ID 必须精确匹配，随后才启动增量同步。
- XState 独占 boot、auth、restore、sync、ready、degraded、offline 和 reconnect 生命周期。
- TanStack Query 只管理控制平面就绪查询；URL 保存大厅、实例、Agent、消息和目录情境。
- 中英文按系统语言初始化，并允许用户显式覆盖。
- 未实现的任务 20–22 页面明确显示交付边界，不伪造 Agent、房间或消息。

## 2. 架构边界

### 2.1 功能模块

- `apps/web/src/features/session/domain`：纯会话状态机、端口和领域结果。
- `apps/web/src/features/session/adapters`：控制平面 HTTP 与 Matrix SDK 适配器。
- `apps/web/src/features/session/ui`：只把状态快照投影为连接舱。
- `apps/web/src/features/health`：就绪报告 schema、查询和健康条。
- `apps/web/src/shared/routing`：URL schema 与规范化。
- `packages/ui-system`：视觉令牌和最小原语，不含业务分支。

React 组件不直接决定认证与恢复规则；Matrix SDK 也不会越过端口进入 JSX。控制平面响应先经 Zod 严格解码，失败进入带边界和恢复动作的 Result，而不是落入永久 loading。

### 2.2 会话与凭据

- 控制平面 OIDC Token 永不进入浏览器 JavaScript；浏览器只持有 `Secure`、`HttpOnly`、`SameSite=Lax` 会话 Cookie。
- Matrix Access Token 只保存在当前标签页 `sessionStorage`，关闭浏览器会话后重新 SSO。
- IndexedDB 只缓存可重建的 Matrix 同步状态。
- Application Service Token 没有进入 Web 依赖、构建产物、响应或存储。
- OIDC 回调显式接受标准 `iss` 与 `session_state`，并对 `iss` 做精确匹配；未知回调字段仍被拒绝。

## 3. 自动验证

### 3.1 静态、单元与构建

```text
corepack pnpm@10.28.0 --filter @agent-room/web typecheck
  通过

corepack pnpm@10.28.0 lint
  通过

corepack pnpm@10.28.0 test
  8 个测试文件，41 个测试通过

corepack pnpm@10.28.0 --filter @agent-room/web build
  通过；PWA 生成成功

cargo test -p agent-room-control-plane features::authentication
  7 个认证 HTTP 测试通过
```

生产构建把 Matrix SDK 和加密 WASM 放在按需分块中。约 7.8 MB 的 WASM 不进入初始连接舱脚本，也不进入当前 PWA 预缓存清单；任务 27 启用 E2EE 时重新评估缓存和下载预算。

### 3.2 真实浏览器壳层验收

```text
corepack pnpm@10.28.0 --filter @agent-room/web test:browser
  3 个 Playwright 测试通过
```

覆盖：

- 桌面 30/70 几何、五段生命周期、键盘跳转和真实 401 未登录态。
- 390 px 移动布局、44 px 主操作和零横向溢出。
- 离线恢复动作和无效深链的明确边界。
- 除审计过的未登录 401 外，无未处理页面异常或控制台错误。

本地产物位于忽略目录 `artifacts/browser/task-19/`；CI 上传 `task-19-browser-evidence`，保留 14 天。

### 3.3 真实身份与 Matrix 联调

为了不删除既有开发卷，本次验收先无损停止 `agent-room-dev`，再使用独立的 `agent-room-task19` Compose 项目启动全新 PostgreSQL、Keycloak、Synapse、Caddy、对象存储和扫描器。随后执行：

```text
python tools/web.py
  1 个真实会话测试通过
```

该测试没有 Mock 身份或 Matrix：

1. 使用隔离开发账户完成 Keycloak 登录。
2. 控制平面回调校验 `state`、PKCE、浏览器 Cookie 和授权服务器 `iss`。
3. Matrix SSO 映射出与控制平面完全一致的稳定 User ID。
4. 浏览器兑换单次 Login Token 并启动真实 `/sync`。
5. 页面刷新后从 `sessionStorage` 凭据和 IndexedDB 同步缓存恢复到 ready。

这次联调实际拦下三处发布级缺陷：CORS 依赖误放在 dev-dependencies、标准 OIDC 回调参数被过度拒绝、IndexedDB Store 在绑定 Matrix Client 前错误启动。三处都已修复并加入回归测试或真实会话门禁。

## 4. 视觉保真账本

| # | 设计约束 | 验收结果 |
| --- | --- | --- |
| 1 | 桌面约 30/70，拒绝传统侧栏与居中卡片 | Playwright 读取实际网格列并断言 0.30；连接轨道与操作面铺满视口 |
| 2 | 当前阶段有编号、符号、标题和说明 | 五段轨道和右侧大号阶段坐标均来自同一 XState 快照 |
| 3 | 故障与恢复动作相邻 | `failure-panel` 紧邻可执行按钮，展示边界、错误码和相关 ID |
| 4 | 健康条只显示真实依赖 | 数据来自 `/health/ready`、Matrix Sync 事件和浏览器网络信号，无随机状态 |
| 5 | 固定色彩语义 | ink/paper/signal/network/alert 分别承载结构、底色、成功、网络和故障；无紫色渐变或玻璃卡片 |
| 6 | 390 px 不退化 | 无横向滚动、主操作可见且触控高度不少于 44 px |
| 7 | 动效与无障碍 | Motion Spring 仅用于阶段切换；reduced-motion、跳转链接、焦点环、文字状态均可用 |
| 8 | 运行时纪律 | 深链与离线有显式边界；真实 SSO 刷新恢复通过；无 Token 写入日志或 URL 残留 |

桌面和移动参考截图分别为：

- `artifacts/browser/task-19/playwright-desktop.png`
- `artifacts/browser/task-19/playwright-mobile.png`

## 5. CI 门禁

- `web-browser` Job 按 Playwright 官方建议安装受控 Chromium 和系统依赖，单 Worker 运行桌面/移动验收并上传证据。
- `integration` Job 在真实依赖启动、数据库迁移和控制平面验证后运行 `python tools/web.py`。
- `playwright.live.config.ts` 同时启动或复用控制平面与 Vite，不依赖人工开终端。
- 本地标准入口为 `just web-browser` 和 `just web-session-integration`。

## 6. 下一边界

任务 20 才会引入 PixiJS 全幅大厅、200 节点性能预算、稳定布局和一一对应的无障碍列表。任务 19 的大厅路由故意只显示未交付边界；在真实大厅投影存在前，任何假节点或演示消息都会污染 Source of Truth。
