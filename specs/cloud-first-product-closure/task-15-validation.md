# 任务 15：自动化回归证据

## 结论

云端优先重构的仓库级自动化门禁已全部通过。测试修复了一个过期契约：人类消息已改由用户 Matrix 会话发送，因此 E2E 不再等待旧的 Agent `Sign and send` 按钮，而是验证 `Send message` 云端发布流程。

## 仓库门禁

以下命令均在 Windows x86-64 本机执行并以退出码 0 完成：

```text
corepack pnpm@10.28.0 format:check
corepack pnpm@10.28.0 lint
corepack pnpm@10.28.0 typecheck
corepack pnpm@10.28.0 test
corepack pnpm@10.28.0 i18n:check
corepack pnpm@10.28.0 protocol:check
corepack pnpm@10.28.0 actions:check
corepack pnpm@10.28.0 secrets:check
corepack pnpm@10.28.0 build
corepack pnpm@10.28.0 audit:node
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo check --workspace
cargo test --workspace
corepack pnpm@10.28.0 --filter @agent-room/web test:browser
corepack pnpm@10.28.0 build:desktop
git diff --check
```

- Vitest：96 个测试文件、365 个测试通过。
- Playwright：30 个浏览器场景通过，包括 390 px、200% 放大、键盘、WCAG 自动扫描和 WebGL 降级。
- Node 依赖审计：没有已知漏洞。
- GitHub Actions 静态检查：59 个 Action 固定到不可变提交；没有启用托管 macOS Runner。
- Rust 全工作区测试退出码为 0。依赖真实 PostgreSQL、Synapse、双 Homeserver 联邦和内容服务的用例保持显式 `ignored`，未被伪报为已执行；它们属于任务 16 的真实环境验收。

## 指定回归场景

| 场景 | 自动化证据 |
| --- | --- |
| 无 Bridge 的 Web | `apps/web/src/app/app-providers.spec.tsx` 验证没有本机 Runtime 仍渲染云端入口。 |
| Bridge 离线的桌面 | `connection-health.spec.ts` 与 `workspace-components.spec.tsx` 验证 Bridge `stopped` 只投影为本机离线，不伪造或阻断云端层。 |
| 多个 Agent 实例 | `agent-fleet.spec.ts` 验证同一 Agent 跨两台设备聚合两个实例，并保留当前设备与降级状态。 |
| 旧协议兼容 | `message-composer.spec.tsx`、`runtime-compatibility.spec.ts` 与 Rust 协议一致性测试验证旧协议只读和旧 actor 投影。 |

## Windows 产物

```text
路径：target/release/bundle/nsis/Agent Room_0.1.0-alpha.7_x64-setup.exe
体积：30,234,354 bytes（28.83 MiB）
SHA-256：DDBFA7E75D167428A8A369309D573A8E6D3C9ED26F198062E96B7E7E61E5D6A0
Authenticode：NotSigned（符合当前 Alpha 不签名策略）
```

Vite 仍报告 Matrix SDK 与主入口 chunk 偏大。这是已知性能债，不影响本次功能闭环或安装器构建；发布文档必须继续披露，后续以路由级拆包单独治理。
