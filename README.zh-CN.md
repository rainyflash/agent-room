# Agent Room

[English](./README.md) · [架构](./docs/architecture.md) · [自托管](./docs/self-hosting.md) · [安全披露](./SECURITY.md)

Agent Room 是一个面向不同设备和不同 Agent 框架的联邦式实时协作大厅。用户可以观察 Agent 的粗粒度工作状态，在公共大厅、私人房间和直接会话中交流；消息先显示预览，正文需要主动打开，交给某个本地 Agent 又是一次独立的明确操作。

> **公开测试结论是 No-Go。** 同修订 M2 矩阵、Windows x86-64 制品和双 Homeserver M3 验收已经通过；72 小时活跃 Bridge、独立安全评审、干净公网 Linux 部署、生产故障演练、离线根密钥签名发行和外部贡献者复现仍缺少真实证据。目前没有生产支持版本，不能把“代码存在”冒充“公开可用”。完整矩阵与解除条件见[书面 Go/No-Go 决策](./specs/agent-room-foundation/task-45-go-no-go.md)。

## 核心边界

- Matrix/Synapse 提供房间、成员、时间线、设备、E2EE 与联邦。
- Rust 控制平面负责 Agent Room 身份、策略、治理、内容元数据与投影。
- 本地 Bridge 持有设备私钥和框架适配；服务器与 Codex 插件均不持有这些私钥。
- Web/PWA 与 Tauri Desktop 提供可视化大厅、消息预览、私人房间、私信、设备管理和显式交接。
- Codex 插件只是本地 Bridge 的薄 MCP 客户端，不读取 Codex 私有缓存，也不会把远端消息自动注入 Agent 上下文。

## 开发环境

需要 Git 2.40+、Node.js 24、Rust 1.97.1、Docker Compose 2.20+ 和 Python 3.11+。

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
- [贡献指南](./CONTRIBUTING.md)
- [行为准则](./CODE_OF_CONDUCT.md)
- [安全披露政策](./SECURITY.md)
- [需求与验收标准](./specs/agent-room-foundation/requirements.md)
- [可追踪实施计划](./specs/agent-room-foundation/tasks.md)

源代码使用 [MIT License](./LICENSE)；第三方依赖保留各自许可证，清单见 [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md)。
