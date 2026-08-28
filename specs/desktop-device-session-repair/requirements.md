# Agent Room 桌面设备会话修复需求

## 1. 问题与范围

Windows Alpha 3 已能安装、启动 Bridge 并完成 OIDC 设备授权，但桌面端仍错误依赖 Web Cookie 会话，且 DesktopShell IPC 缺少读取当前 Agent 身份所需的作用域。这会让用户在浏览器看到授权成功后，桌面端仍停留在降级或启动状态。

本修复覆盖 Windows 桌面端、Bridge IPC、设备身份控制面调用、首次 Agent 引导、生产部署与 Alpha 4 发行。Web/PWA 的既有 Cookie 会话行为必须保持不变。

## 2. 用户故事

### 2.1 桌面授权闭环

作为 Windows 用户，我希望只完成一次浏览器设备授权，返回桌面后即可继续，而不是再次登录 Web 会话。

### 2.2 首个 Agent 自动准备

作为首次使用者，我希望桌面端根据已授权的 Agent Room 账户幂等创建或恢复默认 Agent，并自动把本机 Bridge 绑定到它。

### 2.3 可诊断失败

作为测试用户，我希望网络、权限或配置失败显示稳定错误码与可重试动作，而不是长期停在“正在启动”。

## 3. 验收标准

1. 当 Bridge 仅完成设备授权且尚未配置 Agent 时，桌面监督器应报告“设备已授权、等待 Agent 配置”，不得返回 `bridge.ipc.scope_denied`。
2. 当 DesktopShell 请求当前 Agent 摘要时，IPC 策略应只授予读取自身身份所需的最小作用域，不得授予消息正文或发送权限。
3. 当用户完成 OIDC 设备授权时，桌面连接页应以 Bridge 快照作为身份事实源，不得要求同一 WebView 再建立 Cookie 会话。
4. 当已授权账户尚无 Agent 时，Bridge 应使用设备持有证明调用控制平面，幂等创建默认 Agent；重复请求必须恢复同一个 Agent。
5. 当默认 Agent 已存在时，首次引导应复用它并解析公开大厅目录，不得创建重复 Agent。
6. 当桌面端得到 Agent 与大厅目标时，应持久化 Runtime 目标并受控重启托管 Bridge；重启后应出现 Agent 实例与 Matrix 房间摘要。
7. 当桌面运行于 `http://tauri.localhost` 时，前端不得直接依赖跨源 Web Cookie API；公开 Web/PWA 的精确 Origin 与 CSRF 约束保持不变。
8. 当 Bridge、控制平面或 Matrix 失败时，桌面端应显示稳定错误码、当前阶段和可重试动作。
9. 当 Alpha 4 候选构建完成时，干净安装流程应通过“安装 → 设备授权 → 默认 Agent → Bridge 就绪 → 宿主检测”的真实 WebView 验收。

## 4. 非目标

- 不放宽远程消息正文、发送或任务交接的默认授权。
- 不让 Tauri WebView 读取 Bridge 访问令牌或设备私钥。
- 不把 `http://tauri.localhost` 加入 Web Cookie 管理端点的受信 Origin。
- 不改变 Web/PWA 的 OIDC Authorization Code + PKCE 登录流程。
