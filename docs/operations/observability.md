# 可观测性与告警 Runbook

本 Runbook 只处理可操作故障。Prometheus 和 Grafana 默认仅监听主机回环地址，运营者通过 SSH 隧道访问；不要把 3000 或 9090 暴露到公网。

## SLO 与证据

- 控制平面 API 月度可用性目标：99.9%；服务端 5xx 计为失败，4xx 计为成功处理的客户端请求。
- 交互延迟警戒线：API P95 持续 10 分钟超过 800ms。
- 恢复目标：RPO 取部署配置的 1–15 分钟，RTO 取最近隔离恢复报告声明值。
- Grafana 的 `Agent Room / Service truth` 仪表盘同时展示记录规则和原始依赖、Outbox、投影、Bridge、前端、备份与恢复指标。
- 所有告警必须包含 `impact`、`diagnostic` 与 `runbook_url`。没有明确处置动作的指标只进入仪表盘，不分页。

## 操作入口

```bash
python3 tools/observability.py validate

ssh -L 3000:127.0.0.1:3000 -L 9090:127.0.0.1:9090 operator@HOST
```

首次登录 Grafana 的用户名为 `agent-room-admin`，密码位于状态目录的 `secrets/grafana_admin_password`。不要复制到聊天、工单或 shell 历史。

完整故障演练会逐个停止服务并等待告警触发，然后恢复服务并等待告警清除。只能在已安排维护窗口的生产副本或等价预生产环境执行：

```bash
python3 tools/observability.py drill \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room \
  --target all \
  --confirm-stop-services
```

Bridge 没有服务端容器可供停止，因此脚本对它运行相同规则文件的确定性输入序列；真实桌面断网验收仍需在封闭测试中执行。

## 通用处置纪律

1. 先确认影响范围和告警开始时间，不要先重启所有容器。
2. 使用固定标签 `service`、`severity`、`instance`、`dependency`、`metric` 定位；禁止向指标添加用户、Agent、房间、消息、摘要、Token、URL 或本地路径。
3. 保存 Prometheus 查询和结构化日志中的错误码，不保存消息正文或认证材料。
4. 恢复后确认告警自动清除、用户路径恢复，并记录根因和防复发变更。

## AgentRoomApiAvailabilityFastBurn

检查 `agent_room:api_availability:ratio_5m`、5xx 状态分布与 `agent_room_dependency_available`。若单个依赖不可用，先按对应依赖 Runbook 处置；若控制平面自身持续 5xx，冻结发布、保留失败副本日志并回滚最近部署。恢复标准是 5 分钟可用率回到 99.9% 以上且错误预算不再下降。

## AgentRoomApiLatencyHigh

对比 API P95、数据库池、依赖探测时延与 Outbox 年龄。连接池耗尽时先找慢查询和泄漏，不要盲目扩大池；外部依赖变慢时限流非关键后台工作。恢复标准是 P95 连续 10 分钟低于 800ms。

## AgentRoomDependencyUnavailable

按 `dependency` 定位 PostgreSQL、Matrix、OIDC 或对象存储。确认 DNS、TLS、凭据文件权限和上游健康；不得绕过失败关闭策略。依赖恢复后验证 `/health/ready`、一次真实读取和一次真实写入。

## AgentRoomOutboxBacklog

检查 Matrix 健康、发布器错误和最旧待发布年龄。先修复下游，再观察积压是否单调下降；不要直接删待发布事件。若需要扩容发布器，确认幂等键和顺序约束仍成立。

## AgentRoomOutboxDeadLetters

从结构化日志取得错误码和事件类型，修复不可重试原因后使用受控重放工具。禁止直接修改 `published_at` 冒充成功。重放后核对 Matrix 事件与本地投影一致，并确认死信数归零。

## AgentRoomProjectionStalled

确认 Matrix 同步游标、投影健康状态和更新时间。先停止产生错误投影的消费者，再使用 `projection-rebuild.sh` 在隔离环境验证；生产重建前必须有当期备份。恢复后比较成员数、房间数与权威 Matrix 快照。

## AgentRoomCoreEndpointUnavailable

按 `instance` 检查 `control-plane`、`matrix`、`oidc`、`object-store` 或 `web`。查看对应容器健康和最近部署；只重启确定故障的服务。对象存储恢复后必须验证对象读写，OIDC 恢复后必须验证发现文档和一次登录。

## AgentRoomFederationEndpointUnavailable

分别检查 `/.well-known/matrix/server`、Matrix federation version、DNS A/AAAA 与 TLS 证书链。运行 `python3 tools/production.py federation ...`；不要把内部 Synapse 端口暴露到公网作为修复。

## AgentRoomBridgeAvailabilityLow

查看 Bridge 可用率与重连 P95，按发布版本而非设备标识聚合排查。确认 OIDC device flow、控制平面设备认证和 Matrix 连接；若新版本集中失败，停止自动更新并回滚。指标不得添加设备、用户或 Agent ID。

## AgentRoomPostgresExporterDown

确认 exporter 容器、`agent_room_metrics` 角色、`pg_monitor` 成员资格、TLS 和 `postgres_metrics_password` 文件。该角色只能用于指标，不得复用迁移或应用运行角色。恢复后确认 `up{job="postgres"} == 1`。

## AgentRoomBackupRpoBreached

检查 `agent-room-backup.timer`、最近服务日志、仓库空间、WAL 归档和 `metrics/backup.prom`。不要在失败时先清理旧备份；先保证至少一个已验证恢复点仍在。补做备份后运行摘要验证，确认年龄低于配置 RPO。

## AgentRoomRestoreDrillStale

选择最近已验证备份执行隔离恢复演练，核对数据库、对象、Synapse signing key、OIDC Realm、投影重建和删除账本重放。成功报告和 `metrics/restore.prom` 必须由同一次演练生成。

## AgentRoomRestoreRtoExceeded

按恢复报告拆分数据库 PITR、对象校验、身份恢复和投影重建耗时。优化前保留完整性校验；不能用跳过对象或删除账本重放来伪造更短 RTO。下一次演练必须在相同数据量级下重新证明。

## 数据保留与隐私

- Prometheus 保留 45 天；Alertmanager、Prometheus 与 Grafana 状态使用独立命名卷。
- 前端只上报受限枚举的耗时或分数；不上报 URL、资源名、消息正文、标识符或本地文件路径。
- API 路由使用框架匹配模板，未知方法收敛为 `OTHER`；不得把原始路径或自定义方法写入标签。
- 备份与恢复 textfile 指标仅含时间戳、目标秒数和耗时，目录 `0755`、文件 `0644`，备份正文仍保持私有。
