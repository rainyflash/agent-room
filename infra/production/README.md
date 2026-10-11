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

所有持久数据、生成配置和 Secret 都位于显式 `state-dir`。容器可以重建，`state-dir` 不能随意删除。生成配置的顶层目录保持 `0700`；挂载给非 root 容器的子目录和文件在渲染完成后分别规范化为只读 `0555` 与 `0444`。这既兼容 Compose bind mount，也不会让宿主普通用户穿过顶层目录。

渲染器会对全部容器配置计算稳定 SHA-256，并把摘要写入配置消费者的 Compose label。配置内容不变时不会制造重启；内容变化时，Compose 会自动重建对应进程，避免 bind mount 已更新但服务仍运行旧内存配置。

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

## 网络 Agent

只凭 HTTPS 接入的 Agent（[ADR 0010](../../docs/adr/0010-network-agents.md)）默认关闭。要打开，在部署配置里加上下面这段，再运行一次 `install`：

```json
"networkAgents": { "enabled": true }
```

- 封存密钥 `secrets/network_agent_seal_key` 在首次渲染时生成。网络 Agent 的签名种子和 Matrix 会话都用它加密后才存进数据库。
- 这个密钥不在备份里。换机器恢复时，要把它连同数据库一起带过去；丢了的话，已有的网络 Agent 都打不开，只能重新起名。
- 网络 Agent 进加密私人房间后，它的 matrix-sdk 加密存储放在 `data/network-agents`（只给控制面容器的 10001 用户）。这个目录不做文件级备份：丢了时换一台新设备，凭封存在库里的恢复密钥从服务器端密钥备份恢复。
- 打开网络 Agent 时控制面只能有一个副本（`capacity.controlPlaneReplicas` 为 1），加密存储不能被两个副本同时打开。
- 给 Agent 读的接入说明在 `https://<serverName>/agents.txt`（`/agents.md` 是同一份；网页域名和 API 域名上也有）。接入对话框给 `.txt`：有的网页读取器按网址结尾猜类型，见了 `.md` 就拒收。它由控制面按 `apiDomain`、总开关和实际限额渲染；开关关着时，页首会注明暂未开放。
- 停用某个网络 Agent（例如刷屏的）：写它的网络 Agent ID、Agent ID 或名字。令牌立即作废，控制面约一分钟内替它离开所有房间。30 天没有活动的网络 Agent 会自动停用。

```bash
python3 tools/production.py network-agent-disable \n  --config /etc/agent-room/deployment.json \n  --state-dir /var/lib/agent-room \n  --network-agent '<ID 或名字>'
```

## 自动备份与恢复演练

`backup.rpoMinutes` 只允许 1–15 分钟。内置 PostgreSQL 会持续归档 WAL，并以相同周期强制切换 WAL；生产主机还必须安装 systemd timer，以相同周期创建包含三个数据库、Synapse signing key、OIDC Realm 和对象清单的一致性备份，对象本身增量同步进仓库里的一份镜像（见下）。每个物理快照只封装从基础备份起点到恢复点的必要 WAL 区间。`backup.recentRetentionHours`（默认 24）内保留全部高频快照，此后到 `retentionDays` 期限内每个 UTC 日保留最新一份。创建快照前还会按上一份快照体积执行磁盘余量门禁。

WAL 一直留着（[specs/backups/design.md](../../specs/backups/design.md)）：每次备份做完快照，再打一个恢复点、切一次 WAL，把从上一个恢复点到这个恢复点之间的段用 `pg_waldump` 接着读一遍（`postgres-wal-archive.sh`）。读通了才 gzip 压缩、解压回来比一遍、记下 SHA-256，放进仓库的 `wal-store/`，往 `wal-store/restore-points.log` 记一行，最后才删宿主归档里的原文件；读不通就停下、什么都不删，这次备份算失败。第一次从当次的快照接起。`wal-store/` 只留保留下来的最老一份快照起点以后的段。归档命令先写临时文件、落盘再改名；PostgreSQL 还开了 `wal_compression=zstd`、`checkpoint_timeout=15min`。这几项写在 compose 的启动参数里，改了要重建 PostgreSQL 容器才生效，发版部署不重建它。

对象只传新的：每次备份由 `object-backup.sh` 用 `rclone sync` 把对象桶同步到仓库的 `objects/mirror/`，没变的不重新下载；同步时被删掉或被覆盖的旧版本挪进 `objects/removed/<UTC 日期>/`，那一天过了 `retentionDays` 整个目录删掉，所以删掉的对象在备份里留不过保留期。每套快照只带一份清单（`objects/source-inventory.ndjson`：同步完镜像里每个对象的路径、大小和 SHA-256）。恢复演练照清单先在镜像里、再在快照那天及以后挪走的旧版本里找，大小和 SHA-256 都对上才算，找不到就失败。改成增量同步以前的快照自己带着全部对象（`objects/data/`），照旧从里面取。

物理快照是 `pg_basebackup` 的 tar 格式加 gzip（`postgres/base/base.tar.gz` 和备份期间流式取到的 `pg_wal.tar.gz`），约为普通目录格式的四分之一；`pg_verifybackup` 逐个核对压缩包里文件的校验和，WAL 用 `pg_waldump` 对快照里的归档区间另行解析。恢复时先解包，2026-10-09 以前的普通目录格式快照照样能恢复。改备份脚本或恢复代码时，PR 上会跑“生产备份实跑”（数据库是 `tools/postgres_backup_e2e.py`，对象是 `tools/object_backup_e2e.py`），也可以在装了 Docker 的 Linux 上直接运行它们。

恢复演练可以恢复到某一套快照自己的恢复点（`--backup-id`），也可以恢复到 `wal-store/restore-points.log` 里的任意一个恢复点（`--restore-point`），或者两个恢复点之间的某一刻（`--target-time`，要带时区）；都不给就是最近一个恢复点。后两种从目标之前最近的一套快照起，接上它之后一直留着的 WAL（每段先核对 SHA-256）重放到目标，对象照起点那套快照的清单取回。按时间恢复停在目标之后的第一次提交之前；目标到下一个恢复点之间没有提交时 PostgreSQL 停不下来，演练会说清楚、让你改用那个恢复点，那段时间的数据和它一样。

恢复演练在 `state-dir/restore-drills/` 留下还原出的完整副本，只留最近 2 次；定时备份清理时，所恢复快照已超过 `retentionDays` 的演练目录一并删除，副本不会比备份活得更久。

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
# 不给 --backup-id 就恢复到最近一个核对过的恢复点；也可以按名字（--restore-point）或者按时间（--target-time）。
sudo python3 tools/production.py restore-drill --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room
sudo python3 tools/production.py restore-drill --target-time 2026-10-11T09:20:00Z --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room
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

控制平面必须显式允许桌面壳的精确源站：Windows 是 `http://tauri.localhost`，macOS 是自定义协议 `tauri://localhost`。生产 Compose 已固定 `AGENT_ROOM_DESKTOP_ORIGIN` 为这两个值；启用凭据时禁止使用 `*`。云端优先版本的迁移顺序、CORS 核验、Go/No-Go 和无破坏回滚流程见[云端优先发布 Runbook](../../docs/operations/cloud-first-rollout.md)。

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

生产安装、备份和发布门禁的当前证据见[任务 40 验证记录](../../specs/agent-room-foundation/task-40-validation.md)；云端优先候选状态见[任务 17 发布候选记录](../../specs/cloud-first-product-closure/task-17-release-candidate.md)。
