# Agent Room 生产 Compose

本目录提供单机 Linux 生产参考，以及不改变应用边界的纵向伸缩路径。它不是 Kubernetes 模板，也不会掩盖未完成的公网验收。

## 拓扑

- `gateway`：Caddy TLS、Web 静态资源、API、Matrix 与 OIDC 反向代理；
- `control-plane`：Rust 控制平面，可配置 1–16 个副本；
- `synapse`：Matrix Homeserver，可选 Redis 与 Generic Worker；
- `identity`：优化构建的 Keycloak；
- `postgres`：单机参考数据库，生产增长后可切换外部 PostgreSQL；
- `object-store`：单机 SeaweedFS S3 端点，生产增长后可切换外部 S3；
- `content-scanner`：ClamAV；
- `telemetry`：OpenTelemetry Collector。

所有持久数据、生成配置和 Secret 都位于显式 `state-dir`。容器可以重建，`state-dir` 不能随意删除。

## 主机与 DNS 前置条件

安装脚本要求：

- x86-64 Linux、Docker Engine 与 Compose v2；
- 至少 4 GiB 内存和 20 GiB 可用磁盘，建议 8 GiB 与 100 GiB；
- TCP 80/443 未被占用；
- `serverName`、`appDomain`、`apiDomain`、`matrixDomain`、`identityDomain` 全部解析到该主机；
- 公网能够访问 80/443，以便 ACME 和 Matrix 联邦完成验证。

示例只使用 RFC 保留域名，不能直接部署。

## 首次安装

复制并修改 `deployment.example.json`，然后执行：

```bash
python3 tools/production.py preflight \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room

python3 tools/production.py install \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room
```

`install` 会按顺序完成主机预检、稳定 Secret 生成、Synapse signing key 生成、配置渲染、镜像构建/拉取、数据库启动与迁移、对象桶核验/创建、全栈启动、公开健康检查和联邦委派检查。任何一步失败都会返回非零退出码。

Secret 只通过 Compose Secret 文件挂载。不要把 `/var/lib/agent-room/secrets` 加入 Git、工单或聊天记录。

## 健康、升级与停止

```bash
python3 tools/production.py health --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room
python3 tools/production.py federation --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room
python3 tools/production.py upgrade --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room
python3 tools/production.py down --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room
```

升级前必须先执行任务 41 定义的备份与恢复验证。`down` 只停止容器，不删除 `state-dir`。

## 外部 PostgreSQL 与对象存储

`deployment.external.example.json` 展示外置依赖、控制平面双副本和 Synapse Worker。先运行 `render` 生成稳定 Secret，再由数据库管理员创建以下固定数据库与最小权限角色：

| 数据库 | 所有者/迁移角色 | 运行角色 |
| --- | --- | --- |
| `agent_room` | `agent_room` | `agent_room_runtime` |
| `synapse` | `synapse` | `synapse` |
| `keycloak` | `identity` | `identity` |

外部 PostgreSQL 强制 `require`、`verify-ca` 或 `verify-full`。控制平面运行容器只持有 `agent_room_runtime`，迁移 URL 仅挂载给一次性 `migrate` 容器。

外部对象桶必须由运营者预先创建；初始化容器只执行 `HeadBucket`，不会请求 `CreateBucket` 权限。将外部 S3 凭据写入生成后的 `s3_access_key` 与 `s3_secret_key` 文件，并保持 `0600` 权限。

## 伸缩边界

- `capacity.controlPlaneReplicas > 1`：由 Caddy 对 Compose DNS 返回的控制平面副本负载均衡；
- `capacity.synapseWorkers > 0`：启用 Redis、复制监听和独立 Generic Worker，将同步类请求分配给 Worker；
- 数据库或对象存储需要独立生命周期时，切换为 `external`，不要复制业务数据库逻辑；
- 当前证据不支持引入 Kubernetes。只有多主机调度成为实测瓶颈后才重新评估。

生产安装、备份和发布门禁的当前证据见[任务 40 验证记录](../../specs/agent-room-foundation/task-40-validation.md)。
