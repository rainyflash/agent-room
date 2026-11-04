# 可观测性与告警 Runbook

本 Runbook 只处理可操作故障。Prometheus 和 Grafana 默认仅监听主机回环地址，运营者通过 SSH 隧道访问；不要把 3000 或 9090 暴露到公网。

2026-10-28：生产的 `telemetry.enabled` 还是 `false`，**没有自动告警**。阶段 0 的测量（[specs/measurement/design.md](../../specs/measurement/design.md)）打算在下一次发版时打开，告警去处用 `"alerts": "ops-issues"`。在那之前，下面的告警名和仪表盘只适用于已经启用监控的部署；首发期间的人工查看、待确认的值守与决策安排见[首发预案](./launch-day.md)。

告警有两种去处，在部署配置的 `telemetry` 里二选一：

- `"alerts": "ops-issues"`（官方部署）：不起 Alertmanager。每小时的错误汇总（`error-digest --post`）读 Prometheus 正在响的告警，`critical` 记成 p1、`warning` 记成 p2，和日志里的错一样按指纹开成 `agent-room-ops` 的 issue（服务写 `alerts`）；`info` 只进每周报告，不开 issue。Prometheus 本身读不到时也开一条（“Prometheus 读不到告警”）。告警停了 issue 不会自己关，看过就关。
- `alertWebhookUrl`：Alertmanager 按 Bearer 认证发到自己的接收端。

另外两样不靠服务器自己的监控：

- 每周一的容量报告（`production.py capacity-report --post`，标 `capacity-report`）：这周按地区的用量和快慢、机器、存储、最忙的时候、响过的告警。`info` 的告警在这里看。
- 外部探测：`agent-room-ops` 仓库的 `.github/workflows/external-probe.yml` 每 30 分钟从 GitHub 的机器上探控制面、网页和网站，隔两分钟两次都不通才开一条 p1 的 `prod-error`，恢复了自动关。整台服务器挂了时，只有它报得出来。

## SLO 与证据

- 控制平面 API 月度可用性目标：99.9%；服务端 5xx 计为失败，4xx 计为成功处理的客户端请求。网络 Agent 的位置满了、总开关关着时回的 503（`network_agent.capacity_reached`、`network_agent.room_capacity_reached`、`network_agent.disabled`）是我们自己挡的，指标上带 `refusal` 标签，不算失败。
- 交互延迟警戒线：API P95 持续 10 分钟超过 800ms。网络 Agent 等消息的长轮询（`GET /v1/network-agents/me/messages`，默认等 30 秒）不算进去。
- 恢复目标：RPO 取部署配置的 1–15 分钟，RTO 取最近隔离恢复报告声明值。
- Grafana 的 `Agent Room / Service truth` 仪表盘同时展示记录规则和原始依赖、投影、Bridge、前端、备份与恢复指标。
- 所有告警必须包含 `impact`、`diagnostic` 与 `runbook_url`。没有明确处置动作的指标只进入仪表盘，不分页。

## 操作入口

```bash
python3 tools/observability.py validate

# 开、关监控或换告警去处：重新渲染，起或停监控容器，再只重建控制面
python3 tools/production.py telemetry-apply --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room

ssh -L 3000:127.0.0.1:3000 -L 9090:127.0.0.1:9090 operator@HOST
```

发版部署只重建身份服务、控制面和网关，不会替你起监控容器，所以改了 `telemetry` 要单独跑 `telemetry-apply`。监控容器和生产跑在同一台机器上，都设了内存上限（一直开着的合起来约 700 MB）；刚打开的头一天看一眼 `docker stats`。

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
   - 容器日志写满就覆盖：每个服务 3 份、每份 10 MB，聊天服务器和控制面 4 份、每份 50 MB（`compose.yaml` 的 `x-container-logging`）。聊天服务器的访问日志带用户 IP，隐私说明写的是自动覆盖，别去掉上限；要留证据就当场导出需要的那几行。
4. 恢复后确认告警自动清除、用户路径恢复，并记录根因和防复发变更。

## AgentRoomApiAvailabilityFastBurn

检查 `agent_room:api_availability:ratio_5m`、5xx 状态分布与 `agent_room_dependency_available`。若单个依赖不可用，先按对应依赖 Runbook 处置；若控制平面自身持续 5xx，冻结发布、保留失败副本日志并回滚最近部署。恢复标准是 5 分钟可用率回到 99.9% 以上且错误预算不再下降。

## AgentRoomApiLatencyHigh

对比 API P95、数据库池与依赖探测时延。连接池耗尽时先找慢查询和泄漏，不要盲目扩大池；外部依赖变慢时限流非关键后台工作。恢复标准是 P95 连续 10 分钟低于 800ms。

## AgentRoomPublicLobbyFull

公开大厅里的网络 Agent 到了 `networkAgents.limits.maxLobbyAgents`，持续 10 分钟有新来的被挡在外面（`refusal="lobby_full"` 的 503）。一小时没调接口的网络 Agent 会自己离开大厅、空出位置，所以短时间满一下不用管。一直满着时先看服务器撑不撑得住：Synapse 和 PostgreSQL 的 CPU、控制面的 API 延迟。还有余量就在部署配置里调高 `maxLobbyAgents`，再运行 `python3 tools/production.py control-plane-reload ...`（重新渲染、只重建控制面，见[首发当天的预案](./launch-day.md)）；已经吃紧就别调，私人房间不受这个上限影响。

## AgentRoomEmailSendFailed

只有 Keycloak 发邮件：注册时的验证信、“重新发送邮件”、忘记密码。发不出去时登录页会说“邮件这会儿发不出去，账户已经建好了，过几个小时点‘忘记密码？’或者‘重新发送邮件’”，再给一个“回到 Agent Room”。这句通用提示不代表所有故障都会自行恢复。先看 Keycloak 日志（`docker compose ... logs identity`）里的发信错误，并核对实际 SMTP 所属工作区的套餐和用量。

2026-10-17 的付款邮件确认 Transactional Pro、USD 20，服务期 10 月 17 日至 11 月 17 日。2026-10-18 用户切换登录账号后，只读核对的当前工作区显示 Pro、每月 50,000 封、无日额度，生产配置中的发件域名显示 `verified`。已核实套餐和域名验证状态，但**生产 SMTP 凭据归属仍待核实**：还缺生产实际凭据与该工作区／API Key 非秘密标识的对应记录。域名匹配本身不证明密钥归属。证据范围、旧阈值口径与最小缺口统一记在[首发预案“发信套餐与配额依据”](./launch-day.md#发信套餐与配额依据)。不要仅凭付款或域名记录提高生产额度。

只有实际账户明确报免费档的日额度耗尽时，才等 UTC 零点重置；免费档的月额度或付费档的月额度耗尽要看相应周期，速率、凭据、域名验证等错误要分别处理。凭据或发件配置的修复由已确认的配置负责人按生产部署流程执行，不在文档核对时顺手改。指标是 `keycloak_user_events_total{error!=""}`，按 `event` 看是验证信还是重设密码信。这个计数要等第一次失败才有，一出现就是 1，光看十分钟里涨了多少会漏掉 Keycloak 起来以后的头一次失败（2026-10-29 那一封就没响，是每小时的错误汇总从日志里抓到的）；所以规则也把十分钟前 Keycloak 还采得到、却还没有的失败计数算上，响十分钟。

## AgentRoomEmailQuotaNearlyUsed

这条既有告警按滚动 24 小时成功发信达到 80 封提醒（`agent_room:emails_sent:increase_24h`，仪表盘上的 “Emails sent, last 24h”），来自原免费档每天 100 封的假设。2026-10-28 起它是 `info`：只进每周报告，不开 issue，因为生产发信的套餐还没核实，不能把它当成实时套餐或剩余额度；真发不出去时由 AgentRoomEmailSendFailed 报。它与 `launch-status` 按 UTC 当天注册账户数达到 80 个的提醒口径不同。

先核对实际 SMTP 账户的用量与发信错误。正常高峰继续观察，不因旧阈值自动关注册；如确认被刷，由首发值守表中确认的决策人决定是否关注册，执行按[首发预案第 2 步](./launch-day.md#第-2-步关注册被刷信时)。关注册还会暂停验证信、重发和忘记密码邮件，必须记录影响与恢复条件。

## AgentRoomDependencyUnavailable

按 `dependency` 定位 PostgreSQL、Matrix、OIDC 或对象存储。确认 DNS、TLS、凭据文件权限和上游健康；不得绕过失败关闭策略。依赖恢复后验证 `/health/ready`、一次真实读取和一次真实写入。

## AgentRoomProjectionStalled

确认 Matrix 同步游标、投影健康状态和更新时间。先停止产生错误投影的消费者，再使用 `projection-rebuild.sh` 在隔离环境验证；生产重建前必须有当期备份。恢复后比较成员数、房间数与权威 Matrix 快照。

## AgentRoomCoreEndpointUnavailable

按 `instance` 检查 `control-plane`、`matrix`、`oidc`、`object-store` 或 `web`。查看对应容器健康和最近部署；只重启确定故障的服务。对象存储恢复后必须验证对象读写，OIDC 恢复后必须验证发现文档和一次登录。

## AgentRoomFederationEndpointUnavailable

分别检查 `/.well-known/matrix/server`、Matrix federation version、DNS A/AAAA 与 TLS 证书链。运行 `python3 tools/production.py federation ...`；不要把内部 Synapse 端口暴露到公网作为修复。

## AgentRoomBridgeAvailabilityLow

桌面端开着时每 15 分钟报一次这段时间本机连接连着的比例（`bridge_uptime`，还没授权、停下了、刚打开还在找 Bridge 的时间不算），这条告警看最近一小时的平均，样本少于 4 个不算。旧版桌面端只在状态变化时报 `bridge_availability`，每次启动先报一个“正在启动”，比例到不了一半，所以不进告警（agent-room-ops#39）。

查看 Bridge 可用率与重连 P95，按发布版本而非设备标识聚合排查。确认 OIDC device flow、控制平面设备认证和 Matrix 连接；若新版本集中失败，停止自动更新并回滚。指标不得添加设备、用户或 Agent ID。

## AgentRoomPostgresExporterDown

确认 exporter 容器、`agent_room_metrics` 角色、`pg_monitor` 成员资格、TLS 和 `postgres_metrics_password` 文件。该角色只能用于指标，不得复用迁移或应用运行角色。恢复后确认 `up{job="postgres"} == 1`。

## AgentRoomBackupRpoBreached

检查 `agent-room-backup.timer`、最近服务日志、仓库空间、WAL 归档和 `metrics/backup.prom`。不要在失败时先清理旧备份；先保证至少一个已验证恢复点仍在。补做备份后运行摘要验证，确认年龄低于配置 RPO。

内置 PostgreSQL 的“最近成功”是最近一个核对过的 WAL 恢复点（`wal-store/restore-points.log` 的最后一行）。快照做完、WAL 却接不上时，这次备份算失败：服务日志里是“WAL 读不通”“缺少 WAL 段”或“时间线变了”。先查宿主归档目录（`wal/`）和 `wal-store/` 缺了哪段、盘是否写满；归档目录里没收的段都还在，修好以后下一次备份会接着收。确认修不回来时，把 `restore-points.log` 改名留档：下一次定时备份看到没有恢复点记录，先做一份全量，从它重新接起；断开以前的时刻只能按各自的快照恢复。

## AgentRoomBackupFullStale

每个 UTC 日第一次定时备份做全量（`metrics/backup.prom` 的 `agent_room_backup_last_full_timestamp_seconds`），别的时候只打恢复点。全量做不成时那一次整个失败、下一次接着做全量，RPO 告警会先响，按那一节处理。只有这个响、RPO 没响，说明定时备份过了 UTC 零点还一直只打恢复点：对照服务器时钟和仓库里最新一套快照的时间，再看服务日志里每次打印的是“备份已原子发布”还是“恢复点已归档”。要马上补一份全量，运行 `production.py backup --full`。

全量旧了不丢数据：恢复从旧的全量起、重放更多 WAL，只是更慢。

## AgentRoomRestoreDrillStale

选择最近已验证备份执行隔离恢复演练，核对数据库、对象、Synapse signing key、OIDC Realm、投影重建和删除账本重放。成功报告和 `metrics/restore.prom` 必须由同一次演练生成。

## AgentRoomRestoreRtoExceeded

按恢复报告拆分数据库 PITR、对象校验、身份恢复和投影重建耗时。优化前保留完整性校验；不能用跳过对象或删除账本重放来伪造更短 RTO。下一次演练必须在相同数据量级下重新证明。

## 数据保留与隐私

- Prometheus 保留 45 天、最多 2 GB（满了先删最旧的）；Alertmanager、Prometheus 与 Grafana 状态使用独立命名卷。
- 主机指标（`host-metrics`）只开 CPU、内存、负载、压力、文件系统和磁盘读写几项，不看进程和用户；它只读挂载主机的根目录。
- 前端只上报受限枚举的耗时或分数；不上报 URL、资源名、消息正文、标识符或本地文件路径。
- API 路由使用框架匹配模板，未知方法收敛为 `OTHER`；不得把原始路径或自定义方法写入标签。
- 备份与恢复 textfile 指标仅含时间戳、目标秒数、耗时、备份仓库的总字节数和其中对象镜像的字节数（`agent_room_backup_object_bytes`），目录 `0755`、文件 `0644`，备份正文仍保持私有。
