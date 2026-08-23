# M0 工程地基验证记录

> 验证日期：2026-08-23  
> 结论：通过  
> 对应任务：[实施计划 1–5](./tasks.md#m0工程地基)

## 1. 工具链与统一入口

- Windows PowerShell 5.1 与 PowerShell 7 均能解析全部 `.ps1`。
- `powershell.exe -ExecutionPolicy Bypass -File tools/bootstrap.ps1` 从仓库根目录外也能定位项目，并按锁文件完成依赖安装、工具链检查、协议生成和 Cargo 依赖预取。
- `node tools/bootstrap.mjs` 验证 Git、Rust 1.97.1、Node.js 24、Corepack 固定的 pnpm 10.28.0 和 Docker Compose 2。
- `just check` 是格式、Lint、类型检查、测试、协议一致性和 Secret 扫描的唯一总入口。

## 2. 代码与协议

- Rust Workspace：`cargo fmt`、`cargo check`、Clippy `-D warnings` 和全部测试通过。
- 领域层共 8 个单元/属性测试通过，覆盖容量、租约、身份、内容、授权和上下文交付的非法转换与幂等性。
- TypeScript：ESLint、strict 类型检查和 12 个协议/边界测试通过；协议验证模块的语句、分支、函数和行覆盖率均为 100%。
- Rust 覆盖率门禁为行覆盖率不低于 60%；当前 Workspace 行覆盖率为 63.29%。
- Rust 协议验证器的 2 个一致性测试通过；两种语言共同接受全部正例并拒绝全部恶意反例。
- `pnpm protocol:check` 确认 Rust/TypeScript 生成物与 JSON Schema 一致。

## 3. 真实本地依赖

- Docker Compose 实际启动 PostgreSQL、Synapse、Keycloak、SeaweedFS S3、Mailpit、OpenTelemetry Collector 与 Caddy。
- 健康检查逐个访问真实服务端点，不以容器 `Running` 状态代替服务可用。
- `tools/dev-infra.ps1 seed` 连续执行两次均成功，证明测试用户、Agent、房间和内容对象的创建具备幂等性。
- 常用端口冲突已通过项目专用宿主端口隔离；容器内部继续使用上游标准端口。

## 4. 供应链与安全

- Secret 扫描器自检成功，并扫描 94 个受版本控制的文本文件。
- `pnpm audit:node` 使用 npm 官方审计端点，结果为无已知漏洞。
- `cargo deny check` 的 advisories、bans、licenses 和 sources 全部通过。
- 生成并解析 1 份 Node CycloneDX 1.6 SBOM 与 5 份 Rust CycloneDX 1.5 SBOM。
- SBOM 生成前移除当前进程继承的凭据类环境变量与 `NODE_PATH`，避免构建工具读取无关凭据或发生模块解析投毒。
- GitHub Actions 的第三方 Action 均固定到完整提交 SHA。
