# Agent Room 桌面设备会话修复实施计划

- [x] 1. 修复 DesktopShell IPC 最小权限与状态探测
  - 为 `SelfRead` 增加明确授权与拒绝矩阵测试。
  - 区分设备已授权与 Agent Runtime 已就绪。
  - _需求：1、2、8_

- [x] 2. 增加设备身份默认 Agent 用例
  - 应用层抽取按 Principal 幂等确保默认 Agent 的共享逻辑。
  - 控制平面增加带设备持有证明的端点及测试。
  - _需求：4、5_

- [x] 3. 增加 Bridge 首次引导 IPC
  - Bridge 组合设备会话、默认 Agent 与公开大厅目录。
  - IPC 只返回可展示和配置的非秘密摘要。
  - _需求：3、4、5_

- [x] 4. 完成桌面 Runtime 目标绑定
  - Tauri 命令调用 Bridge bootstrap、持久化目标并受控重启。
  - 重试复用同一 Agent 与目录目标。
  - _需求：5、6、8_

- [ ] 5. 分离 Web 与 Desktop 会话组合根
  - 桌面连接页只消费 Bridge 快照。
  - Web/PWA 保持 Cookie 与浏览器 Matrix 会话。
  - _需求：3、7、8_

- [ ] 6. 完成多层回归与真实 WebView 验收
  - 运行 Rust、TypeScript、组件、Playwright 与安装后诊断。
  - 验证桌面模式不再请求 `/auth/session`。
  - _需求：1—9_

- [ ] 7. 发布 Alpha 4
  - 提交版本、构建 Windows 候选并发布 prerelease。
  - 部署兼容控制平面变更，完成生产健康与安装验证。
  - _需求：9_
