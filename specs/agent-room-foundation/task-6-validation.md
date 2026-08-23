# 任务 6 控制平面验证记录

> 验证日期：2026-08-23  
> 结论：通过  
> 对应任务：[实施计划 6](./tasks.md#m1内部纵向切片)

## 1. 组合根与依赖方向

- Axum 只在进程组合根组装路由、配置、探针、日志和优雅关闭；应用层健康服务仅依赖 `DependencyProbe` 端口。
- PostgreSQL、Matrix 和对象存储探针是基础设施适配器，通过依赖注入接入 `ReadinessService`。
- PostgreSQL 连接池惰性创建；进程能启动后再由 Ready 探针如实报告依赖状态。
- 启动配置是强类型的，密码调试输出固定脱敏；无效端口、URL、TLS 模式和超时范围会立即失败。

## 2. HTTP 与可观测性契约

- `GET /health/live` 只报告进程存活。
- `GET /health/ready` 并发检查 PostgreSQL、Matrix 和对象存储；完全就绪返回 `200`，任一降级返回 `503`。
- `GET /capabilities` 返回由协议 Schema 生成的 `CapabilityManifest`，没有手写第二份 DTO。
- 上游合法 `x-correlation-id` 会保留；缺失或非法值会替换为 UUIDv7，并在响应头、结构化 JSON 日志和 W3C Trace Context 中串联。
- 可选 OTLP/HTTP Protobuf 导出器只由显式配置启用；日志不记录查询串、凭据或底层客户端原始错误。

## 3. 真实依赖断连矩阵

`python tools/control-plane.py test` 使用本地 Docker 服务实际运行下列矩阵，不用 Mock 伪造结论：

| 场景 | HTTP | PostgreSQL | Matrix | 对象存储 |
| --- | ---: | --- | --- | --- |
| 全部正常 | 200 | ready | ready | ready |
| PostgreSQL 地址不可达 | 503 | degraded | ready | ready |
| Matrix 地址不可达 | 503 | ready | degraded | ready |
| 对象存储地址不可达 | 503 | ready | ready | degraded |

独立进程冒烟还验证了 Live、Ready、Capability 和 UUIDv7 响应头；`Ctrl+C` 能触发优雅关闭并回收 SQLx 连接池。

## 4. 自动化与质量结果

- `just check` 通过：Rust fmt、Clippy `-D warnings`、Cargo check/测试、TypeScript 格式/类型/构建/测试、协议生成一致性、Secret 扫描和 GitHub Actions SHA 固定全部通过。
- Rust Workspace 行覆盖率为 63.52%，通过不低于 60% 的门禁；TypeScript 协议验证包行、语句、分支和函数覆盖率均为 100%。
- `cargo deny check` 的 advisories、bans、licenses 和 sources 通过；重复版本保持可见警告。
- `pnpm audit:node` 未发现已知漏洞。
- CI 已增加真实控制平面断连验收，与本地共用同一个 Python 编排入口。

## 5. 已知非阻断项

- Windows MSVC 链接器会把本地化的“正在创建库”输出标为 `linker_messages` 警告；这不是编译、Clippy 或运行故障。
- `cargo-deny` 报告的重复间接依赖来自 SQLx、OpenTelemetry 和跨平台 TLS 树，当前策略为警告而非禁止；后续升级上游时继续收敛。
