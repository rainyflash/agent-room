# 公开测试阻断执行 Runbook

本文只描述仍未完成的真实外部门禁。唯一状态源是
[`release/go-no-go/public-beta.json`](../../release/go-no-go/public-beta.json)；GitHub Issue 只是由
`python tools/readiness_issues.py sync --repo rainyflash/agent-room` 生成的执行视图。

## 共同证据规则

- 每份报告必须记录精确 Git SHA、UTC 时间、工具版本、拓扑和退出状态。
- 报告只允许脱敏后的结构化指标，不得包含凭据、消息正文、隐藏推理、用户目录或临时令牌。
- 截图、口头确认、缩短时长、空闲进程和跨修订拼接都不能关闭阻断。
- 原始证据进入受访问控制的不可变存储；仓库只提交脱敏摘要及其 SHA-256。
- 完成一项后先更新唯一状态源、重新生成 Go/No-Go，再运行 Issue 同步器。不得直接手工关闭 Issue。

## GNG-002：独立安全与隐私评审

责任人：未参与实现的安全评审方和隐私评审方。开发者不能给自己签字。

评审输入至少包含 `security.md`、`SECURITY.md`、ADR、协议 Schema、E2EE/设备恢复实现、提示注入证明、部署模型、数据删除边界和任务 35 原始安全报告。评审必须覆盖：

1. 身份、设备、恢复、Matrix E2EE 与密钥生命周期；
2. 私聊/私人房间访问控制、联邦治理与重放；
3. Bridge/Codex 显式交付边界、提示注入与本机权限；
4. SSRF、上传扫描、对象访问、日志/指标/审计隐私；
5. 账户导出、删除残留和独立 Homeserver 无法强制删除的诚实文案；
6. 最小范围黑盒渗透测试。

合格报告必须绑定 Git SHA、列出范围和未覆盖项，并由评审方标注每个发现的严重度、复现步骤与处置结论。所有阻断/高危项修复并复验前不得关闭本项；中低风险接受必须有负责人和复核日期。

## GNG-003：72 小时活跃 Bridge 与容量汇总

责任人：容量测试运营者。必须使用真实已授权、已登录且已进入大厅的会话：

```bash
python tools/capacity_bridge.py \
  --duration-seconds 259200 \
  --sample-seconds 30 \
  --agent-id <UUIDv7> \
  --catalog-id <UUIDv7>
```

保持机器不休眠、网络和磁盘监控开启。完成后在同一精确修订依次重跑五个真实场景：

```bash
python tools/capacity_database.py
python tools/capacity_matrix.py --sustained-seconds 60
python tools/object_store.py
python tools/capacity_federation.py --outage-seconds 1800 --events 10
python tools/capacity_web.py

python tools/capacity.py gate \
  artifacts/capacity/database-report.json \
  artifacts/capacity/matrix-report.json \
  artifacts/capacity/content-report.json \
  artifacts/capacity/federation-report.json \
  artifacts/capacity/web-report.json \
  artifacts/capacity/bridge-soak-report.json
```

关闭条件：六份报告具有同一 SHA、`passed=true`、`releaseGateEligible=true`；Bridge 最大 RSS 不超过 512 MiB，增长不超过 128 MiB。72 小时不能压缩或模拟。

## GNG-004：干净公网 Linux 部署

责任人：持有公网主机、域名和 DNS 权限的部署运营者。使用全新 x86-64 Linux 主机，不复用开发机缓存：

```bash
git clone https://github.com/rainyflash/agent-room.git
cd agent-room
git checkout <已审查候选 SHA>

just doctor
node tools/bootstrap.mjs
python3 tools/self_host.py init \
  --domain room.example.com \
  --output /etc/agent-room/deployment.json \
  --backup-repository <异地加密备份仓库>

python3 tools/self_host.py doctor \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room
python3 tools/self_host.py install \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room
python3 tools/self_host.py health \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room
python3 tools/self_host.py federation \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room
```

`room.example.com` 只是占位符，实际执行必须使用运营者控制的真实域名。关闭条件：DNS、ACME、OIDC 登录、对象读写、公开 Matrix 委派、两台独立 Homeserver 双向事件/回执全部从公网验证；数据库、Redis、对象管理口和监控端口不得暴露公网。

随后安装备份计划，并用同一次真实备份完成校验和隔离恢复：

```bash
python3 tools/self_host.py backup-schedule-install --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room
python3 tools/self_host.py backup --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room
python3 tools/self_host.py backup-verify --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room --backup-id <BACKUP_ID>
python3 tools/self_host.py restore-drill --config /etc/agent-room/deployment.json --state-dir /var/lib/agent-room --backup-id <BACKUP_ID>
```

## GNG-005：生产等价副本故障演练

责任人：运行与可观测性负责人。只能在已安排维护窗口的生产等价副本执行：

```bash
python3 tools/observability.py validate
python3 tools/observability.py drill \
  --config /etc/agent-room/deployment.json \
  --state-dir /var/lib/agent-room \
  --target all \
  --confirm-stop-services
```

关闭条件：控制平面、Matrix、对象存储、OIDC、Gateway/联邦和 Bridge 六域均观察到预期分页告警、Runbook 处置、服务恢复与告警清除；`artifacts/observability/fault-drill-report.json` 与 Prometheus 原始查询一致。不要在真实生产主站无维护窗口地执行。

## GNG-006：离线根密钥与受保护发行

责任人：至少两名发行维护者。严格执行[签名发布 Runbook](./signed-releases.md)：

1. 在断网、全盘加密的专用设备从已审查 SHA 构建 `agent-room-release-tool`；
2. 生成根密钥，制作两个分别保管的加密离线副本，只有公钥离开离线区；
3. 配置受保护的 `release-candidate` 与 `public-release` Environment，并要求不同维护者审批；
4. 完成 Windows 原生候选、三个多架构 OCI、逐件 SBOM/Sigstore、数据库扩展和兼容服务端晋级；
5. 离线签署根清单，再由最终工作流复验并发布；
6. 在 testing 渠道实际演练正常升级、下载中断恢复和带更高序号的显式回滚。

私钥不得进入 GitHub Secret、联网开发机、同步盘、容器或聊天记录。没有真实双人仪式和受保护环境记录就不能关闭本项。

## GNG-007：外部干净复现

责任人：未参与项目开发的贡献者与独立部署运营者。

贡献者在全新系统账户或 VM 中执行：

```bash
git clone https://github.com/rainyflash/agent-room.git
cd agent-room
just doctor
node tools/bootstrap.mjs
just check
just oss-acceptance
```

独立运营者再按 GNG-004 从公开仓库完成部署，全程不得手工修改数据库。合格证据是公开 Issue/PR：包含 OS/架构、精确 SHA、各命令退出状态、发现的问题及修复提交；不得上传 `.env.local`、Secret 文件或用户内容。

## 状态收敛

所有阻断证据进入版本控制后执行：

```bash
python tools/go_no_go.py validate
python tools/go_no_go.py generate
python tools/readiness_issues.py sync --repo rainyflash/agent-room
```

只有 `public-beta.json` 中所有阻断均关闭、要求全为通过且生成记录一致时，才允许把结论改为 Go。
