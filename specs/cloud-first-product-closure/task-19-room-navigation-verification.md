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

升级后曾出现旧 STDIO 连接返回 `Transport closed`。本次后续复验中，已安装 Alpha 15 的 Bridge 与 MCP 已恢复 `ready`，真实在线状态、消息预览和按需正文均可读取。桌面窗口当前显示“需要登录操作者账户”；仍需操作者完成登录，才能完成以下最终可视化验收：

`工作区 → 房间目录 → Agent Room Global → 消息预览 → 按需正文`

这一步完成前，任务 19.5 保持未完成。

## 新发现的会话生命周期缺陷

Alpha 15 桌面 WebView 复用 `MatrixWebGateway`，Matrix Access Token 保存在 `sessionStorage`。它满足刷新恢复，但不能作为跨进程恢复契约，导致升级后再次请求 Matrix SSO。控制平面人类会话和 Bridge 设备会话已经进入 Windows Credential Manager，Matrix 人类设备会话此前尚未进入原生存储。

本次源码新增 `MatrixSessionVault` 端口：浏览器实现继续使用标签页级 `sessionStorage`；桌面 Adapter 使用三个固定 Tauri 命令连接系统凭据库。系统凭据库保护静态存储，不代表运行中的 Matrix SDK 不再需要读取 Access Token；不能把它描述成消除了所有 WebView 风险。

## 任务 20：持久会话实现与验证

架构职责：

- `MatrixSessionRepository` 负责凭据轮换、串行落盘、失败重试和退出代际隔离，不依赖 DOM 或 Tauri。
- `MatrixWebGateway` 只负责 Matrix 认证、身份核对、加密初始化及同步连接；重建网关复用原有 `userId`、`deviceId` 和设备加密库。
- `MatrixCredentialRuntime` 使用环境和 Homeserver 派生稳定命名空间，存储严格版本化的会话记录；前端不能指定任意凭据名称。
- 新令牌保存失败时不会宣称成功；当前进程暂存最新令牌，下次恢复先重试保存。退出清理会排在已经开始的存储操作之后，并拒绝过期请求再次写回。
- [Matrix 刷新协议](https://spec.matrix.org/v1.16/client-server-api/#post_matrixclientv3refresh)允许省略新的刷新令牌和有效期。实现按协议保留旧刷新令牌，不依赖 SDK 42 将可选字段写成必填的类型声明。
- 原生命令的对象、JSON 字符串和 Error 包装使用同一错误解析器；界面提供凭据库不可用、记录损坏及桌面版本不匹配的中英文恢复指引。

已通过的验证：

- TypeScript：103 个测试文件、413 项测试通过；类型检查与 ESLint 通过。
- Rust 桌面：53 项测试通过；Clippy 全目标检查通过。
- Windows 凭据库：使用随机测试命名空间写入合成凭据，由另一个独立进程恢复相同用户和设备，随后清理测试记录；未读取或改写真实用户凭据。
- 浏览器：`/connect → 凭据库故障 → 重试 → Matrix 登录入口` 通过，1440×1000 和 390×844 无稳定横向溢出；无页面异常或框架覆盖层。使用现有 Playwright 工作流，因为本会话没有旧 Browser 插件及其 `browser` 技能；测试中的 Tauri 和服务端响应是明确注入的故障夹具，不冒充真实登录结果。
- 桌面前端生产构建通过；构建仍有既有 Matrix/Pixi 大分块提示，未作为本次修复扩大处理。
- 真实已安装 Alpha 15：MCP 返回 `ready`，Agent 在线状态为 `idle`；已有“Alpha 14 连接验收”预览及对应 87 字节正文均可读取。内容摘要为 `ab656730eb4778c6fbd6e9662345e3adc2387f22504e807d5b915d4641465327`。

交付边界：本次持久会话修复尚未发布或替换已安装 Alpha 15。没有启动新的 GitHub Actions 打包任务，没有运行 macOS Runner。生产账户的完整登录、升级后重开和肉眼可见的大厅正文验收尚未完成，任务 19.5、20.6 继续保持未勾选。
