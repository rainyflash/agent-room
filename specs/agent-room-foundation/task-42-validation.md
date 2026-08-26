# 任务 42 验证记录：可观测性、SLO 与故障处置

## 1. 当前结论

任务 42 的工程实现已经完成：应用、控制平面、数据库、Matrix、OIDC、对象存储、Bridge、前端、联邦、备份和恢复均进入同一条低基数观测链路；13 个分页告警全部由 `promtool` 输入序列触发验证；Grafana 仪表盘直接引用同一记录规则与原始指标。

任务清单暂不勾选完成，因为真实生产副本上的六域停止/恢复演练仍依赖任务 40 的公网 Linux 部署。工具已经就绪，但没有运行环境就声称通过属于伪造证据。

## 2. 指标架构

- Rust 控制平面通过 OpenTelemetry 记录固定路由 API 请求量、延迟、依赖可用性与探测耗时；
- 周期采样器从 PostgreSQL 读取连接池、Outbox、投影、内容回收和账户删除积压；
- Web 与 Desktop 只上报受限枚举的 Web Vitals、场景初始化、消息显式打开、Bridge 可用率和重连耗时；
- Synapse 与 Keycloak 使用原生 Prometheus 入口；PostgreSQL 使用独立 `agent_room_metrics` + `pg_monitor` 角色；
- Blackbox Exporter 探测固定命名的核心与联邦端点；
- 备份和恢复工具原子写入 Node Exporter textfile 指标；
- OTel Collector 只暴露 Prometheus 拉取端点，Prometheus 保留 45 天并向 Alertmanager 路由。

指标标签不允许用户、主体、Agent、房间、事件、消息、Token、摘要、URL、文件名或本地路径。HTTP 使用框架匹配路由模板；任意扩展方法收敛为 `OTHER`。

## 3. SLO 与仪表盘

记录规则提供：

- `agent_room:api_availability:ratio_5m`；
- `agent_room:api_availability:ratio_30d`；
- `agent_room:api_latency:p95_5m`；
- `agent_room:api_error_budget_remaining:ratio_30d`。

目标为月度 API 可用性 99.9%，P95 警戒线 800ms。Grafana `Agent Room / Service truth` 还展示核心/联邦探测、依赖、Outbox、投影、Bridge、前端 P95、备份 RPO 与恢复 RTO。Grafana 和 Prometheus 只绑定主机回环地址。

## 4. 分页告警

13 个告警覆盖：API 快速燃烧、API 延迟、核心依赖、Outbox 积压/死信、投影停滞、核心端点、联邦端点、Bridge 群体可用率、PostgreSQL exporter、备份 RPO、恢复演练新鲜度和恢复 RTO。

每条告警都包含影响、诊断和 Runbook URL；没有明确处置动作的指标不分页。启用 telemetry 时部署配置必须提供无凭据 HTTPS 告警接收端，Bearer Secret 由安装器独立生成。

## 5. 自动门禁

```text
python tools/observability.py validate
  Prometheus 18 条记录/告警规则语法通过
  13 个分页告警输入序列模拟通过
  仪表盘必需 SLO 与恢复查询通过
  标签隐私契约通过

python -m unittest tools.tests.test_observability tools.tests.test_prodops
  22 项通过

python tools/production.py render ...
docker compose ... config --quiet
  生产渲染与 Compose 结构通过

docker manifest inspect ...
  Prometheus、Alertmanager、Blackbox、Postgres Exporter、Node Exporter、Grafana 固定镜像标签存在
```

## 6. 待执行真实演练

在任务 40 的等价预生产或生产副本中执行：

```bash
python3 tools/observability.py drill \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room \
  --target all \
  --confirm-stop-services
```

脚本逐个停止 Control Plane、Matrix、内置对象存储、OIDC 和 Gateway，等待对应告警触发，再恢复服务并等待告警清除；Bridge 使用同一规则文件的确定性故障序列。真实桌面 Bridge 断网仍由任务 36 封闭测试补充。报告写入 `artifacts/observability/fault-drill-report.json`。

处置步骤、恢复标准与隐私纪律见[可观测性 Runbook](../../docs/operations/observability.md)。
