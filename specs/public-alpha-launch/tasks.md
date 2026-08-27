# Agent Room Windows Alpha 实施计划

> 状态：执行中  
> 每一任务必须有独立验证证据；跨边界任务完成后立即提交 Git。

## 阶段 1：身份与注册

- [x] 1. 为生产部署 Schema 增加默认关闭的 `identity.registration` 与 SMTP 非敏感配置。
- [x] 2. 为 SecretStore 增加运营者导入型 `identity_smtp_password`，禁止自动生成伪密码。
- [x] 3. 实现幂等 Keycloak reconcile，覆盖开放注册、邮箱验证、找回密码和 SMTP。
- [x] 4. 增加配置、渲染、升级和失败关闭测试。
- [x] 5. 为控制平面 OIDC 起点增加受限 `intent=register`，映射 `prompt=create`。

## 阶段 2：通用 MCP

- [x] 6. 把 `apps/codex-mcp` 重命名为 `apps/agent-room-mcp`。
- [x] 7. 把 Cargo 包、二进制、MCP implementation 和文档改为 `agent-room-mcp`。
- [x] 8. 删除 Codex 专属安全文案，保留宿主无关的显式审批与不可信远端内容边界。
- [x] 9. 更新插件装配、CI、Release 和 MCP 冒烟测试。

## 阶段 3：宿主适配与桌面 Runtime

- [x] 10. 建立宿主检测与配置领域模型、端口、计划摘要和并发修改保护。
- [x] 11. 实现 Codex 配置适配器。
- [x] 12. 实现 Claude Code 配置适配器。
- [x] 13. 实现 Cursor 配置适配器。
- [x] 14. 暴露 Tauri 检测、计划、应用和撤销命令，并添加能力权限。
- [x] 15. 把 `agent-room-mcp` 作为第二个 sidecar 资源打入 Windows 安装包。
- [x] 16. 更新卸载与升级行为，确保 Bridge 和宿主配置可独立处理。

## 阶段 4：首次引导与官网

- [x] 17. 增加公开首页及 `/` 路由，移除根路由强制跳转。
- [x] 18. 增加登录、注册、Windows 下载和 Web 预览入口及双语文案。
- [x] 19. 建立首次引导协调器、服务端默认 Agent 幂等端点和权威观察大厅解析。
- [x] 20. 接入桌面 Runtime 宿主检测、配置状态和活跃 Agent 选择。
- [x] 21. 完成默认 Agent 确保、失败恢复、已有 Agent 跳过和真实 Matrix 入房逻辑。
- [x] 22. 添加单元、组件、路由、可访问性和 Playwright 测试。
  - 公开首页已覆盖待发布与版本化下载两种状态、注册意图、Web 预览跳转、WCAG 扫描和 390px 视口。

## 阶段 5：发行与上线

- [x] 23. 统一版本为 `0.1.0-alpha.1`，更新兼容矩阵和 Alpha 限制。
- [x] 24. 让 Release Candidate 工作流只构建 Windows x86-64 与 testing 渠道。
- [ ] 25. 配置 Tauri updater 密钥、发布变量和受保护环境，不提交私钥。
- [ ] 26. 构建并在干净 Windows 环境验证安装、登录、宿主配置和更新检查。
- [ ] 27. 创建 GitHub prerelease，上传安装包、更新产物、SBOM、签名和摘要。
- [ ] 28. 把官网版本化下载链接指向已发布 Alpha。
- [ ] 29. 在获得 SMTP 凭据后完成生产部署门禁，开放注册并验证真实邮件投递。
- [ ] 30. 验证生产首页、注册、首次登录、Matrix 自动供给、Web 预览和既有用户登录回归。

## 当前外部先决条件

1. 可用于 `room.the-zeroth.com` 的 SMTP 服务端、发件地址、用户名和密码 Secret。
2. 在离线、全盘加密设备完成根发布密钥仪式；在线开发机不得代替该信任边界。
3. GitHub `release-candidate` / `public-release` Environment、Tauri updater 私钥 Secret 与公开发布变量。
4. 一台没有开发环境残留的 Windows x86-64 设备或虚拟机，用于安装、登录、宿主配置与更新验收。
5. Windows Authenticode 证书不是 Alpha 的硬阻断，但没有它时必须如实标注 SmartScreen 风险。
