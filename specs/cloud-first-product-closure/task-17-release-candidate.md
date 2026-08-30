# 任务 17：云端优先发布候选记录

## 结论

云端优先重构的源码、自动化回归和真实 Windows/Tauri Bridge-offline 验收已经完成。**线上部署与下一版 Windows Alpha 发布尚未执行**；它们需要用户明确确认，并且必须按发布 Runbook 先云端、后客户端。

## 已完成候选能力

- Web 与桌面端共享同一云端路由、端口和领域模型；
- Web 不依赖 Bridge，桌面端在 Bridge 离线时只降级本机能力；
- 用户会话与 Bridge Agent 凭据隔离；
- 多账号、多设备、公共大厅、消息和实例定向交接真实闭环；
- 生产 Compose 显式允许精确 Windows Tauri 源站 `http://tauri.localhost`；
- README、架构、兼容矩阵、已知限制、手动 MCP 和故障诊断已同步。

## 验证证据

- 浏览器三上下文：同账号双设备、第二账号、跨账号大厅消息；
- 真实 Windows Tauri/WebView2：Bridge 指向不可达端口时，控制平面与 Matrix 在线、工作区可见、公共大厅可进入；
- 桌面脱敏产物：`artifacts/desktop-acceptance/desktop-cloud-closure.json`（本地产物，不提交）；
- 设计与实现分项证据：`task-16-5-validation.md`、`task-16-6-browser-validation.md`、`task-16-6-desktop-validation.md`。

## 2026-08-30 发布候选门禁

- `cargo fmt --all --check`、全工作区 Clippy `-D warnings`、`cargo check` 与 `cargo test` 通过；
- Prettier、ESLint、i18n、TypeScript、Web production build 与协议生成一致性通过；
- Vitest：96 个测试文件、371 项测试全部通过；
- Python：222 项通过，6 项按环境契约跳过；
- Secret 扫描：871 个文本文件通过；
- GitHub Actions 策略：59 个远程 Action 全部固定，且没有托管 macOS Runner；
- 第三方许可证：1600 个锁定依赖版本与清单一致；
- 开源检查：29 份文档与 2 份 Issue 模板通过；
- Go/No-Go 记录结构有效，结论仍为预期的 `NO-GO`，因为稳定版外部阻塞没有被本次 Alpha 重构伪造为完成；
- 真实 Windows Tauri/WebView2 验收再次通过：Bridge `halted`、控制平面与 Matrix `online`、工作区可见并进入 `Vertical Codex Lobby`。

Vite 仍报告 Matrix crypto WASM 和多个 JavaScript Chunk 的体积告警。它不影响本次正确性闭环，但已作为显式性能债写入已知限制，不能把警告藏在构建日志里。

## 发布前阻塞

当前线上环境不能被视为已具备新桌面客户端条件。发布前必须验证生产 API 对 `Origin: http://tauri.localhost` 返回精确 `Access-Control-Allow-Origin` 和 `Access-Control-Allow-Credentials: true`。未完成生产备份/恢复演练、迁移和该预检时，禁止发布新 Windows 客户端。

## 版本策略

当前公开版本仍为 `0.1.0-alpha.7`。候选源码的版本号与下载链接只在实际发布事务中一次性升级，避免 README 指向不存在的资产。发布必须创建更高的 prerelease 版本与单调递增签名序列，不能覆盖旧资产。

## 执行入口

部署、观察、Go/No-Go 和回滚严格遵循[云端优先版本发布 Runbook](../../docs/operations/cloud-first-rollout.md)。收到明确确认前，本任务停在发布门前是正确状态，不是实现缺失。
