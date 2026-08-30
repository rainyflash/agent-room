# 任务 17：云端优先发布记录

## 结论

云端优先重构已于 2026-08-30 完成生产迁移，并以 `v0.1.0-alpha.8` 发布 Windows testing prerelease。生产 Control Plane、Matrix、Web、精确 CORS、PWA 更新提示、公开安装器、运行中升级、Bridge 和九工具 MCP 均通过真实环境验收。

本记录只把本次 Alpha 发布事务标记为完成。稳定版 Go/No-Go 仍受离线根密钥仪式、外部复现、双独立 Homeserver、长期容量观察和外部安全审查约束，不能偷换成稳定版已就绪。

## 不可变发布边界

- 发布标签：`v0.1.0-alpha.8`；
- 发布提交：`f8754a5ae1bee195bf07d28f44476663b60c02cd`；
- testing 单调序号：`8`；
- 公开 Release：<https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.8>；
- 生产仓库 `/opt/agent-room` 干净且固定在同一提交；
- 该提交相对已部署服务提交的差异只涉及 Codex 插件契约，运行中的 Control Plane 仍报告 `0.1.0-alpha.8`。

## 生产迁移证据

- 生产备份 `20260830T143046796843Z-02e64b53` 完成清单与载荷验证；
- 隔离恢复演练通过，耗时 `6.744` 秒；
- 向前兼容迁移 `202608300001_desktop_human_sessions` 与 `202608300002_targeted_handoffs` 已应用，既有公共大厅迁移保留；
- `GET /health/ready` 返回 `ready`，PostgreSQL、Matrix 与对象存储依赖均为 `ready`；
- Matrix Client Versions 端点返回有效版本集合；
- `http://tauri.localhost` 与 `https://app.room.the-zeroth.com` 预检均返回精确 `Access-Control-Allow-Origin` 和 `Access-Control-Allow-Credentials: true`；`https://evil.example` 不返回 Allow-Origin；
- 备份定时器保持 `active`；部署配置权限保持 `0600 root:root`；
- 发布后最终镜像 ID：Control Plane `sha256:ad69084b4b5a448f77d325248bda9b2d7a2bb357ed1eea68b532d8eb18dc05b9`、Gateway `sha256:786af1f06c9f49452aa000559ba6093bf6ee91f78ecf7ec60d6ab2d91657533c`、Identity `sha256:05f3224fb8381eedd95b41f6a641e3a70b1b33e48e480b5f9549a913f3eba9e8`；
- 官网下载指针从 `alpha.6` 原子切换到 `alpha.8`，切换前配置备份为 `/etc/agent-room/deployment.json.pre-alpha8-download-20260830T163731Z.bak`。

Release 内的 `database-expanded-evidence.json`、`compatible-server-evidence.json` 和两份晋级记录使用 SHA-256 串联数据库扩展与兼容服务阶段；最终发布工作流追加 `release-promotion-clients-published.json`。

## Windows 公开资产验收

- 候选工作流：<https://github.com/rainyflash/agent-room/actions/runs/33320844968>；
- 最终发布工作流：<https://github.com/rainyflash/agent-room/actions/runs/33322419339>；
- 安装器：`agent-room-installer-v0.1.0-alpha.8-windows-x86_64.exe`；
- 字节数：`30,350,612`；
- SHA-256：`6d70bd497cb7060ceee35ae9178385103381f90dc201722b258f7d96e25c4adf`；
- 干净 Windows 候选机验证静默安装、Desktop/Bridge/MCP 启动、运行中升级、三个旧进程收敛、升级后重新启动与静默卸载全部通过；
- 本机从 GitHub 公开 Release 重新下载同一摘要安装器，完成 `0.1.0-alpha.7 → 0.1.0-alpha.8` 原地升级；
- 新版 Desktop 启动后 Bridge 正常运行；已安装 MCP 真实返回 9 个工具，包含 `agent_room_list_handoffs`，并成功调用 `agent_room_get_self`；
- 安装器必须停止宿主正在使用的旧 MCP 进程，因此已打开的 Codex 任务会得到 `Transport closed`，重启 Codex 后由宿主重新建立连接。这是进程升级边界，不是 Bridge 或 MCP 验收失败。

## Web 与 PWA 验收

- 服务器当前 JavaScript 资产包含版本化 `alpha.8` 安装器 URL；
- 已有 Chrome 配置先由旧 Service Worker 展示 `alpha.6`，随后明确出现“已验证的新版本可用”；
- 点击“重新加载更新”后，下载按钮切换到 `alpha.8`，更新提示消失；
- 未登录访问 `/workspace` 会停在显式认证边界，不会伪造本地会话。

## 质量门禁与费用修复

- 发布提交主 CI `33320837266` 与 CodeQL `33320837235` 通过；服务部署基线的主 CI `33318726498`、CodeQL `33318726161` 和联邦验收 `33318726478` 通过；
- 候选首次运行 `33319153399` 在完成昂贵 Windows 构建后发现 Codex 插件仍声明 8 个工具，未创建标签或 Release；
- 修复提交把第九个 `agent_room_list_handoffs` 纳入插件与审批契约，第二次候选完整通过；
- 后续发布不再维护独立 Python 工具列表：Rust `#[tool(name = ...)]` 是工具集合唯一来源，廉价静态门禁在占用原生构建资源前校验审批策略与风险标注，构建后仍保留真实 `tools/list` 冒烟；
- Python 回归共 226 项通过、6 项按环境契约跳过；Secret 扫描通过 871 个文本文件；
- GitHub Actions 仍未配置任何托管 macOS Runner。

Vite 仍报告 Matrix crypto WASM 和多个 JavaScript Chunk 的体积告警。它不影响本次正确性闭环，但继续作为显式性能债记录，不能藏在构建日志里。

## 回滚点

- 数据库只执行加法迁移，回滚应用时不得删除新增表或列；
- 生产备份与恢复演练已经验证；
- Web 可用上述配置备份恢复旧下载指针；
- Windows 资产不可覆盖或删除；发现客户端缺陷时必须发布更高 testing 序号的前向修复；
- `v0.1.0-alpha.7` 保留为上一公开客户端兼容点。
