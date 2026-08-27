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
- `telemetry`：OpenTelemetry Collector；
- `prometheus`、`alertmanager`、`blackbox` 与 exporter：SLO、依赖和恢复事实；
- `grafana`：仅通过主机回环地址访问的运营仪表盘。

所有持久数据、生成配置和 Secret 都位于显式 `state-dir`。容器可以重建，`state-dir` 不能随意删除。

## 主机与 DNS 前置条件

安装脚本要求：

- x86-64 Linux、Docker Engine 与 Compose v2；
- 至少 4 GiB 内存和 20 GiB 可用磁盘，建议 8 GiB 与 100 GiB；
- TCP 80/443 未被占用；
- `serverName`、`appDomain`、`apiDomain`、`matrixDomain`、`identityDomain` 全部解析到该主机；
- 公网能够访问 80/443，以便 ACME 和 Matrix 联邦完成验证。

内存预检允许固件与内核最多保留 256 MiB，因此真实的 4 GiB 云主机即使在 Linux 中显示略少也会通过；可见内存低于 3.75 GiB 仍会被拒绝。

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

Secret 只通过 Compose Secret 文件挂载。父目录保持 `0700`，单个文件规范化为只读 `0444`，以兼容非 Swarm Compose 对非 root 容器的 bind-mount 语义；宿主普通用户仍无法穿过父目录。不要把 `/var/lib/agent-room/secrets` 加入 Git、工单或聊天记录。

`telemetry.enabled=true` 时必须配置不含凭据的 HTTPS `alertWebhookUrl`。安装器会生成独立 Bearer Secret；告警接收端必须支持 `Authorization: Bearer`。Grafana 和 Prometheus 默认只监听 `127.0.0.1:3000` 与 `127.0.0.1:9090`，通过 SSH 隧道访问。完整验证与故障演练见[可观测性 Runbook](../../docs/operations/observability.md)。

## 自动备份与恢复演练

`backup.rpoMinutes` 只允许 1–15 分钟。内置 PostgreSQL 会持续归档 WAL，并以相同周期强制切换 WAL；生产主机还必须安装 systemd timer，以相同周期创建包含三个数据库、Synapse signing key、OIDC Realm、对象清单和对象字节的一致性备份。这里选择完整快照是有意的：公开测试阶段先保证可恢复性，不拿未经验证的“增量优化”冒充 RPO。

先渲染并审查 unit，再以 root 安装和核验：

```bash
python3 tools/production.py backup-schedule-render \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room

sudo python3 tools/production.py backup-schedule-install \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room

sudo python3 tools/production.py backup-schedule-verify \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room
```

安装后的 unit 指向当前源码目录、配置文件和状态目录，因此源码部署目录必须保持稳定。备份服务以非重叠 oneshot 运行；失败会让 unit 进入 failed 状态，不能被脚本吞掉。使用 `systemctl status agent-room-backup.timer` 和 `journalctl -u agent-room-backup.service` 接入任务 42 的告警。

手工备份、摘要核验、保留清理和隔离恢复演练：

```bash
sudo python3 tools/production.py backup --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room
sudo python3 tools/production.py backup-verify --backup-id BACKUP_ID --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room
sudo python3 tools/production.py restore-drill --backup-id BACKUP_ID --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room
sudo python3 tools/production.py backup-prune --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room
```

外部 PostgreSQL 不允许伪装成本地 PITR：配置必须引用 30 分钟内采集、声明 RPO 不高于部署目标的供应商证据，真实隔离恢复仍由供应商流程执行。备份仓库必须位于独立故障域并由运营者另行加密；放在应用主机同一块磁盘只算副本，不算灾备。

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

| 数据库       | 所有者/迁移角色 | 运行角色                                |
| ------------ | --------------- | --------------------------------------- |
| `agent_room` | `agent_room`    | `agent_room_runtime`                    |
| `synapse`    | `synapse`       | `synapse`                               |
| `keycloak`   | `identity`      | `identity`                              |
| `postgres`   | —               | `agent_room_metrics`（仅 `pg_monitor`） |

外部 PostgreSQL 强制 `require`、`verify-ca` 或 `verify-full`。控制平面运行容器只持有 `agent_room_runtime`，迁移 URL 仅挂载给一次性 `migrate` 容器。
外部数据库管理员还必须创建 `agent_room_metrics` 登录角色、授予 `pg_monitor`，并把密码写入生成后的 `postgres_metrics_password` 文件；不得给该角色数据库所有权或迁移权限。

外部对象桶必须由运营者预先创建；初始化容器只执行 `HeadBucket`，不会请求 `CreateBucket` 权限。将外部 S3 凭据写入生成后的 `s3_access_key` 与 `s3_secret_key` 文件；再次运行安装器会把它们规范化为父目录 `0700`、文件 `0444` 的容器 Secret 权限模型。

## 伸缩边界

- `capacity.controlPlaneReplicas > 1`：由 Caddy 对 Compose DNS 返回的控制平面副本负载均衡；
- `capacity.synapseWorkers > 0`：启用 Redis、复制监听和独立 Generic Worker，将同步类请求分配给 Worker；
- 数据库或对象存储需要独立生命周期时，切换为 `external`，不要复制业务数据库逻辑；
- 当前证据不支持引入 Kubernetes。只有多主机调度成为实测瓶颈后才重新评估。

生产安装、备份和发布门禁的当前证据见[任务 40 验证记录](../../specs/agent-room-foundation/task-40-validation.md)。
