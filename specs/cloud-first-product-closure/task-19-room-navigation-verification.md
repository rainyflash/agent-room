# 任务 19：房间目录与大厅导航验收

## 根因与修复

账号工作区此前只展示云端设备与 Agent 舰队；真实消息层仅挂载在大厅路由中，但 Router 没有公共房间目录，工作区也没有大厅入口。消息已经通过 Matrix 发布，用户却没有任何产品路径可以抵达它。这是导航闭环缺失，不是空状态文案问题。

Alpha 15 完成以下修复：

- 把公共房间目录从首次引导中拆成独立 Feature，领域选择、Control Plane Adapter、TanStack Query 和 UI 各自保持单一职责；
- 增加 `/rooms` 路由，严格解析 `GET /lobbies/public`，并提供加载、空状态、错误恢复与中英文排序；
- 在 `/workspace` 顶栏增加明确的大厅入口；
- 连接完成动作准确命名为“打开账户工作区”，不再谎称直接进入大厅；
- 增加桌面、窄屏、键盘导航、无横向溢出和真实入厅路由回归。

## 自动化证据

- 仓库质量门禁：`100` 个测试文件、`386` 个测试通过；
- 浏览器验收：`32` 个 Playwright 场景通过；
- 房间目录在桌面和窄屏真实渲染中均无控制台错误；
- 生产 `GET /lobbies/public` 返回的公共大厅契约与 Alpha 15 客户端严格 Schema 一致；
- 测试消息 `Agent Room 端到端测试 2026-09-04` 已写入 Matrix 公共大厅，并能由 Bridge 最小预览链路读取。

## 不可变发布边界

- 公开预发行版：<https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.15>；
- 发布提交：`9f1ea02850dbba3d0346859ab0326c396618b1ff`；
- 候选工作流：<https://github.com/rainyflash/agent-room/actions/runs/33841938566>；
- 最终发布工作流：<https://github.com/rainyflash/agent-room/actions/runs/33844083371>；
- Windows 安装器：`agent-room-installer-v0.1.0-alpha.15-windows-x86_64.exe`；
- 安装器字节数：`30,486,708`；
- 安装器 SHA-256：`76c235d5dfe9846cd86f51454638999e25f35071c66aebf6cc558a557087193f`。

候选机已用该安装器完成原地升级，已安装版本为 `0.1.0-alpha.15`。最终发布门禁重新验证了 testing 渠道签名、全部摘要、SBOM、Sigstore 身份、兼容服务证据和客户端发布证据；本次没有运行托管 macOS Runner。

## 尚待完成的真实桌面链路

升级会终止旧 MCP 进程，因此当前 Codex 任务中的旧 STDIO 连接按设计返回 `Transport closed`；宿主重启后会连接 Alpha 15 MCP。桌面主窗口已经进入原生 Matrix SSO 回调流程，仍需操作者在系统浏览器完成密码登录，然后才能完成以下最终人工验收：

`工作区 → 房间目录 → Agent Room Global → 消息预览 → 按需正文`

这一步完成前，任务 19.5 保持未完成。

## 新发现的会话生命周期缺陷

桌面 WebView 当前复用 `MatrixWebGateway`，Matrix Access Token 保存在 `sessionStorage`。它满足刷新恢复，但安装器关闭进程后无法恢复，导致升级后再次请求 Matrix SSO。控制平面人类会话和 Bridge 设备会话已经进入 Windows Credential Manager，Matrix 人类设备会话尚未进入原生安全存储。

后续必须新增桌面专用 `MatrixSessionVault` 端口，由 Tauri Adapter 使用系统凭据库持久化，浏览器实现继续保持标签页级 `sessionStorage`。不能粗暴改成 `localStorage`；那会把长期 Matrix Token 暴露给 WebView 脚本，属于用便利性制造凭据泄漏面。
