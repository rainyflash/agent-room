# Agent Room Windows Alpha 实施计划

> 状态：执行中  
> 每一任务必须有独立验证证据；跨边界任务完成后立即提交 Git。

## 阶段 1：身份与注册

- [ ] 1. 为生产部署 Schema 增加默认关闭的 `identity.registration` 与 SMTP 非敏感配置。
- [ ] 2. 为 SecretStore 增加运营者导入型 `identity_smtp_password`，禁止自动生成伪密码。
- [ ] 3. 实现幂等 Keycloak reconcile，覆盖开放注册、邮箱验证、找回密码和 SMTP。
- [ ] 4. 增加配置、渲染、升级和失败关闭测试。
- [ ] 5. 为控制平面 OIDC 起点增加受限 `intent=register`，映射 `prompt=create`。

## 阶段 2：通用 MCP

- [ ] 6. 把 `apps/codex-mcp` 重命名为 `apps/agent-room-mcp`。
- [ ] 7. 把 Cargo 包、二进制、MCP implementation 和文档改为 `agent-room-mcp`。
- [ ] 8. 删除 Codex 专属安全文案，保留宿主无关的显式审批与不可信远端内容边界。
- [ ] 9. 更新插件装配、CI、Release 和 MCP 冒烟测试。

## 阶段 3：宿主适配与桌面 Runtime

- [ ] 10. 建立宿主检测与配置领域模型、端口、计划摘要和并发修改保护。
- [ ] 11. 实现 Codex 配置适配器。
- [ ] 12. 实现 Claude Code 配置适配器。
- [ ] 13. 实现 Cursor 配置适配器。
- [ ] 14. 暴露 Tauri 检测、计划、应用和撤销命令，并添加能力权限。
- [ ] 15. 把 `agent-room-mcp` 作为第二个 sidecar 资源打入 Windows 安装包。
- [ ] 16. 更新卸载与升级行为，确保 Bridge 和宿主配置可独立处理。

## 阶段 4：首次引导与官网

- [ ] 17. 增加公开首页及 `/` 路由，移除根路由强制跳转。
- [ ] 18. 增加登录、注册、Windows 下载和 Web 预览入口及双语文案。
- [ ] 19. 建立首次引导领域状态机和服务端 Agent 查询/创建网关。
- [ ] 20. 接入桌面 Runtime 宿主检测与配置状态。
- [ ] 21. 完成创建第一个 Agent、失败恢复和已有 Agent 跳过逻辑。
- [ ] 22. 添加单元、组件、路由、可访问性和 Playwright 测试。

## 阶段 5：发行与上线

- [ ] 23. 统一版本为 `0.1.0-alpha.1`，更新兼容矩阵和 Alpha 限制。
- [ ] 24. 让 Release Candidate 工作流只构建 Windows x86-64 与 testing 渠道。
- [ ] 25. 配置 Tauri updater 密钥、发布变量和受保护环境，不提交私钥。
- [ ] 26. 构建并在干净 Windows 环境验证安装、登录、宿主配置和更新检查。
- [ ] 27. 创建 GitHub prerelease，上传安装包、更新产物、SBOM、签名和摘要。
- [ ] 28. 把官网版本化下载链接指向已发布 Alpha。
- [ ] 29. 在获得 SMTP 凭据后完成生产部署门禁，开放注册并验证真实邮件投递。
- [ ] 30. 验证生产首页、注册、首次登录、Matrix 自动供给、Web 预览和既有用户登录回归。

## 当前外部先决条件

1. 可用于 `room.the-zeroth.com` 的 SMTP 服务端、发件地址、用户名和密码 Secret。
2. GitHub `release-candidate` / `public-release` Environment 与 updater 签名变量。
3. Windows Authenticode 证书不是 Alpha 的硬阻断，但没有它时必须如实标注 SmartScreen 风险。
