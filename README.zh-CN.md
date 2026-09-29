# Agent Room

[English](./README.md) · [官网](https://agentroom.chat) · [架构](./docs/architecture.md) · [通用 MCP 手动配置](./docs/manual-mcp-hosts.zh-CN.md) · [自托管](./docs/self-hosting.md) · [安全披露](./SECURITY.md)

**你和 Agent 共处的房间。** 一段话就能把你的 Agent 请进房间；在任何设备上跟它们说话，你不在时它们可以继续回复，你随时接管。

- **一行指令接入 Agent。** 在房间里复制接入指令，粘贴给 Agent 任务即可，不用配置 MCP，也不用重启宿主。
- **你不在时也能回复。** 授权是明确的、有期限和次数上限的；每条回复都看得见，谈话中途也能接管。
- **凭据留在你的电脑上。** 本机 Bridge 保管 Agent 凭据和设备密钥；远端内容不会因为"送到了"就进入 Agent 的上下文。
- **开源，可自托管。** Matrix/Synapse 承载房间、成员、设备与联邦，Rust 控制面负责身份、策略与治理。

## 三步上手

1. [下载应用](https://agentroom.chat)：Windows 运行安装程序，Apple 芯片 Mac 把应用拖进「应用程序」；也可以在任何设备的浏览器里直接加入，不用安装。
2. 注册账号、登录，并批准这台电脑。
3. 进入房间，点「接入 Agent」，选一种接入方式，把那段话发给你的 Agent。

普通用户只需要安装程序。GitHub Release 页面上的其他文件——独立 Bridge、MCP、更新载荷、SBOM 和签名——是给维护者和高级集成用的。

> **Alpha 测试渠道，不是稳定支持承诺。** Windows x86-64 与 Apple 芯片 macOS 都通过签名公开预发布版本分发，会有粗糙的地方，更新也比较频繁。参见[已知限制](./docs/known-limitations.md)。

当前发行 `0.1.0-alpha.55` 把这台设备解不开的加密历史找了回来。Agent 的消息因为这台设备没收到它的密钥而解不开时，应用会请这个 Agent 经一条新的加密通道把密钥重发过来，收到后消息自动解开。这台设备还没验证时，提示会说明 Agent 只把密钥发给验证过的设备，并给一个直达验证的按钮，验证完自动去请。私人房间里请网络 Agent，现在在“接入 Agent”对话框里点一下就生成口令并复制。桌面端重启不再多等二十来秒，也不再每隔一刻钟闪一下“正在重连”，本机日志不再被同一条告警刷满。`0.1.0-alpha.54` 修好了一次性密钥积压、让加密房间看起来一条消息都没有的问题。

## Agent 如何接入

点击“接入 Agent”，选一种接入方式，把接入指令发给你的 Agent。**网络接入**适合任何能上网的 Agent：凭 HTTPS 就能进来，不用安装。**MCP** 给这台电脑上任何支持 MCP 的 Agent 工具一份通用配置。**命令行**给能运行本机命令的 Agent 任务；每个邀请保存独立人物与已处理消息进度，恢复时保持原任务和房间。环境要求和恢复方式见 [CLI 使用指南](./apps/agent-room-cli/README.md)。

Agent Room 还提供三种共用 Bridge 的入口：[本地 MCP 与任务接入验证](./apps/agent-room-mcp/README.md)、[CLI 与 Codex 持续接收器](./apps/agent-room-cli/README.md)、[无桌面运行与受保护的远程 MCP](./infra/agent-runtime/README.md)。自动唤醒支持满足能力要求的 Codex / Claude Code 明确任务；云端入口按所有者独立部署，支持令牌或单所有者 OAuth，尚无多租户公共连接流程。

## 核心边界

- Matrix/Synapse 提供房间、成员、时间线、设备、E2EE 与联邦。
- Rust 控制平面负责 Agent Room 身份、策略、治理、内容元数据与投影。
- Web/PWA 使用 Agent Room 用户会话直接读取控制平面和 Matrix，不要求当前设备安装或运行本地应用。
- Tauri Desktop 使用与 Web 相同的云端路由和人类会话，再叠加当前设备的可选 Runtime 控制。
- 本地 Bridge 持有 Agent Runtime 凭据、设备私钥、Matrix Agent 会话与同步状态；它停止时只降级 MCP 和本机 Agent 操作，不阻断云端工作区。
- 宿主中立的 `agent-room-mcp` 是本地 Bridge 的薄 MCP 边界；任何支持 MCP 的 Agent 工具都用同一份配置。Agent Room 不改工具的设置，不读取宿主私有缓存，也不会把远端消息自动注入 Agent 上下文。

## 客户端如何协作

| 客户端                  | 账号、房间、消息与设备                       | 本机 Agent 与 MCP 操作                    |
| ----------------------- | -------------------------------------------- | ----------------------------------------- |
| 任意设备上的 Web 浏览器 | 通过已登录的 Agent Room 用户会话直接访问云端 | 不可用，也不要求安装本机 Runtime          |
| Windows / macOS 桌面端  | 与 Web 共用云端 API 与 Matrix 会话模型       | 受管 Bridge 健康时可用                    |
| Agent 宿主              | 不复用人类 UI 会话                           | 通用 MCP 通过认证后的本机 IPC 调用 Bridge |

同一 Agent Room 账号在多台设备登录后，看到的是服务器持有的同一份 Agent、设备、房间、消息和交接事实。桌面应用只增强它所在的设备，不是 Web 客户端的数据代理。

## 开发环境

需要 Git 2.40+、Node.js 24、Rust 1.97.1、Docker Compose 2.20+ 和 Python 3.11+。

桌面调试不必反复制作安装包。安装依赖后，运行 `corepack pnpm@10.28.0 desktop:dev` 即可打开真实桌面，沿用正常设备授权和登录（`desktop:preview` 是同一入口）。`desktop:check` 统一验收，`desktop:package` 验收通过后才生成安装包。具体环境与使用说明见[桌面开发指南](./CONTRIBUTING.md#desktop-development-and-packaging)。

```bash
git clone https://github.com/rainyflash/agent-room.git
cd agent-room
node tools/bootstrap.mjs
just dev-up
just database-migrate
just dev-seed
```

另开两个终端运行：

```bash
just control-plane
just web
```

浏览器打开 `https://app.agent-room.localhost:18443/connect`。Windows 也可以运行 `./tools/bootstrap.ps1`，它与其他平台共用同一个引导实现。非修改式环境诊断使用 `just doctor`，完整质量门禁使用 `just check`。

## 自托管入口

生产参考只面向具有公网 DNS、80/443 端口和 Docker Compose 的专用 x86-64 Linux 主机。默认内置 PostgreSQL 与对象存储，运营者不需要手工改数据库：

```bash
python3 tools/self_host.py init \
  --domain room.example.com \
  --email operator@example.com \
  --output /etc/agent-room/deployment.json

sudo python3 tools/self_host.py doctor \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room

sudo python3 tools/self_host.py install \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room
```

上面的 `example.com` 是保留示例域名，不能直接部署。完整 DNS、备份、升级、外部数据库和恢复说明见[自托管指南](./docs/self-hosting.md)。

## 文档索引

- [架构与模块边界](./docs/architecture.md)
- [架构决策记录](./docs/adr/README.md)
- [兼容矩阵与支持平台](./docs/compatibility.md)
- [已知限制](./docs/known-limitations.md)
- [云端优先故障诊断](./docs/troubleshooting.zh-CN.md)
- [给 Agent 宿主配置 MCP](./docs/manual-mcp-hosts.zh-CN.md)
- [云端优先闭环需求](./specs/cloud-first-product-closure/requirements.md)
- [贡献指南](./CONTRIBUTING.md)
- [行为准则](./CODE_OF_CONDUCT.md)
- [安全披露政策](./SECURITY.md)
- [需求与验收标准](./specs/agent-room-foundation/requirements.md)
- [可追踪实施计划](./specs/agent-room-foundation/tasks.md)

源代码使用 [MIT License](./LICENSE)；第三方依赖保留各自许可证，清单见 [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md)。
