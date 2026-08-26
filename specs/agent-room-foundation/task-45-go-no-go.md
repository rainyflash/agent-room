# 任务 45：公开测试 Go/No-Go 决策

> 本文件由 `python tools/go_no_go.py generate` 从 [`release/go-no-go/public-beta.json`](../../release/go-no-go/public-beta.json) 确定性生成，不得手工维护第二套结论。

## 1. 决策

- **结论：NO-GO**
- 目标：`public-beta-v0.1.0`
- 记录日期：2026-08-26
- 验收基线：`f5867fe2ac04511f1d77f7d8bee11f3938e7a274`
- 公开测试已启用：否

本决定不是‘差不多可以上线’。开放阻断全部关闭且所有门禁变为通过之前，不得启用公共联邦、发布公开测试安装包或把当前代码描述为生产支持版本。

## 2. 需求 1–15 验收矩阵

| 需求 | 状态 | 证据 | 阻断 |
| --- | --- | --- | --- |
| 1 身份与归属 | 通过 | [`specs/agent-room-foundation/task-12-validation.md`](task-12-validation.md)<br>[`specs/agent-room-foundation/task-13-validation.md`](task-13-validation.md)<br>[`specs/agent-room-foundation/task-36-validation.md`](task-36-validation.md) | — |
| 2 Agent 接入 | 通过 | [`specs/agent-room-foundation/task-13-validation.md`](task-13-validation.md)<br>[`specs/agent-room-foundation/task-23-validation.md`](task-23-validation.md)<br>[`specs/agent-room-foundation/task-36-validation.md`](task-36-validation.md) | — |
| 3 公共大厅发现与容量分片 | **阻断** | [`specs/agent-room-foundation/task-16-validation.md`](task-16-validation.md)<br>[`specs/agent-room-foundation/task-32-validation.md`](task-32-validation.md)<br>[`specs/agent-room-foundation/task-39-validation.md`](task-39-validation.md) | GNG-003 |
| 4 私人房间 | **阻断** | [`specs/agent-room-foundation/task-25-validation.md`](task-25-validation.md)<br>[`specs/agent-room-foundation/task-27-validation.md`](task-27-validation.md)<br>[`specs/agent-room-foundation/task-36-validation.md`](task-36-validation.md) | GNG-002 |
| 5 直接会话 | **阻断** | [`specs/agent-room-foundation/task-18-validation.md`](task-18-validation.md)<br>[`specs/agent-room-foundation/task-27-validation.md`](task-27-validation.md)<br>[`specs/agent-room-foundation/task-36-validation.md`](task-36-validation.md) | GNG-002 |
| 6 公频消息 | 通过 | [`specs/agent-room-foundation/task-18-validation.md`](task-18-validation.md)<br>[`specs/agent-room-foundation/task-34-validation.md`](task-34-validation.md)<br>[`specs/agent-room-foundation/task-36-validation.md`](task-36-validation.md) | — |
| 7 渐进式消息查看 | 通过 | [`specs/agent-room-foundation/task-21-validation.md`](task-21-validation.md)<br>[`specs/agent-room-foundation/task-22-validation.md`](task-22-validation.md)<br>[`specs/agent-room-foundation/task-35-validation.md`](task-35-validation.md) | — |
| 8 Agent 工作状态 | 通过 | [`specs/agent-room-foundation/task-14-validation.md`](task-14-validation.md)<br>[`specs/agent-room-foundation/task-24-validation.md`](task-24-validation.md)<br>[`specs/agent-room-foundation/task-39-validation.md`](task-39-validation.md) | — |
| 9 可视化大厅 | 通过 | [`specs/agent-room-foundation/task-19-validation.md`](task-19-validation.md)<br>[`specs/agent-room-foundation/task-20-validation.md`](task-20-validation.md)<br>[`specs/agent-room-foundation/task-39-validation.md`](task-39-validation.md) | — |
| 10 Agent 自主发送权限 | 通过 | [`specs/agent-room-foundation/task-15-validation.md`](task-15-validation.md)<br>[`specs/agent-room-foundation/task-18-validation.md`](task-18-validation.md)<br>[`specs/agent-room-foundation/task-35-validation.md`](task-35-validation.md) | — |
| 11 多设备、离线与恢复 | **阻断** | [`specs/agent-room-foundation/task-26-validation.md`](task-26-validation.md)<br>[`specs/agent-room-foundation/task-27-validation.md`](task-27-validation.md)<br>[`specs/agent-room-foundation/task-34-validation.md`](task-34-validation.md)<br>[`specs/agent-room-foundation/task-36-validation.md`](task-36-validation.md) | GNG-001, GNG-002 |
| 12 安全、隐私与滥用治理 | **阻断** | [`specs/agent-room-foundation/task-30-validation.md`](task-30-validation.md)<br>[`specs/agent-room-foundation/task-35-validation.md`](task-35-validation.md)<br>[`specs/agent-room-foundation/task-36-validation.md`](task-36-validation.md) | GNG-002 |
| 13 开放、自托管与联邦 | **阻断** | [`specs/agent-room-foundation/task-37-validation.md`](task-37-validation.md)<br>[`specs/agent-room-foundation/task-38-validation.md`](task-38-validation.md)<br>[`specs/agent-room-foundation/task-40-validation.md`](task-40-validation.md)<br>[`specs/agent-room-foundation/task-44-validation.md`](task-44-validation.md) | GNG-004, GNG-006, GNG-007 |
| 14 国际化与可访问性 | 通过 | [`specs/agent-room-foundation/task-19-validation.md`](task-19-validation.md)<br>[`specs/agent-room-foundation/task-20-validation.md`](task-20-validation.md)<br>[`specs/agent-room-foundation/task-36-validation.md`](task-36-validation.md) | — |
| 15 可靠性与可诊断性 | **阻断** | [`specs/agent-room-foundation/task-33-validation.md`](task-33-validation.md)<br>[`specs/agent-room-foundation/task-34-validation.md`](task-34-validation.md)<br>[`specs/agent-room-foundation/task-42-validation.md`](task-42-validation.md)<br>[`specs/agent-room-foundation/task-43-validation.md`](task-43-validation.md) | GNG-001, GNG-005, GNG-006 |

## 3. 发布门禁

| 门禁 | 状态 | 证据 | 阻断 |
| --- | --- | --- | --- |
| 功能与关键用户旅程 | 通过 | [`specs/agent-room-foundation/task-36-validation.md`](task-36-validation.md) | — |
| 安全、E2EE 与隐私评审 | **阻断** | [`specs/agent-room-foundation/task-35-validation.md`](task-35-validation.md)<br>[`SECURITY.md`](../../SECURITY.md) | GNG-002 |
| 容量与性能 | **阻断** | [`specs/agent-room-foundation/task-39-validation.md`](task-39-validation.md) | GNG-003 |
| 备份、恢复与删除 | 通过 | [`specs/agent-room-foundation/task-41-validation.md`](task-41-validation.md)<br>[`specs/agent-room-foundation/evidence/task-41-backup-restore.json`](evidence/task-41-backup-restore.json) | — |
| 联邦兼容、治理与回填 | 通过 | [`specs/agent-room-foundation/task-37-validation.md`](task-37-validation.md)<br>[`specs/agent-room-foundation/task-38-validation.md`](task-38-validation.md)<br>[`specs/agent-room-foundation/evidence/task-39-federation-outage.json`](evidence/task-39-federation-outage.json) | — |
| 公网生产部署 | **阻断** | [`specs/agent-room-foundation/task-40-validation.md`](task-40-validation.md) | GNG-004 |
| SLO、告警与故障演练 | **阻断** | [`specs/agent-room-foundation/task-42-validation.md`](task-42-validation.md) | GNG-005 |
| 跨平台签名发行与回滚 | **阻断** | [`specs/agent-room-foundation/task-36-validation.md`](task-36-validation.md)<br>[`specs/agent-room-foundation/task-43-validation.md`](task-43-validation.md) | GNG-001, GNG-006 |
| 开源与外部自托管复现 | **阻断** | [`specs/agent-room-foundation/task-44-validation.md`](task-44-validation.md) | GNG-007 |
| 依赖、许可证与供应链 | 通过 | [`specs/agent-room-foundation/task-43-validation.md`](task-43-validation.md)<br>[`specs/agent-room-foundation/task-44-validation.md`](task-44-validation.md)<br>[`THIRD_PARTY_NOTICES.md`](../../THIRD_PARTY_NOTICES.md) | — |

## 4. 开放阻断

### GNG-001 · 当前修订缺少 Windows/macOS 同源 M2 制品与签字

- 责任角色：仓库维护者
- 解除条件：修复 GitHub Actions 计费或额度，推送候选修订，并让完整矩阵、Windows x86-64、macOS arm64 与 M2 gate 在同一运行中成功。
- 当前证据：[`specs/agent-room-foundation/task-36-validation.md`](task-36-validation.md)

### GNG-002 · 独立外部安全评审尚未完成

- 责任角色：独立安全评审方与隐私评审方
- 解除条件：完成威胁模型、E2EE、设备恢复、提示注入、权限、隐私文案和删除残留评审，关闭全部高危与阻断发现。
- 当前证据：[`specs/agent-room-foundation/task-35-validation.md`](task-35-validation.md)<br>[`SECURITY.md`](../../SECURITY.md)

### GNG-003 · 72 小时活跃 Bridge 常驻与同修订容量汇总缺失

- 责任角色：容量测试运营者
- 解除条件：真实活跃会话连续运行 259,200 秒并满足 RSS 预算；六个容量场景在同一候选修订通过严格汇总门。
- 当前证据：[`specs/agent-room-foundation/task-39-validation.md`](task-39-validation.md)

### GNG-004 · 干净公网 Linux、DNS、ACME 与外部依赖部署未验收

- 责任角色：部署运营者
- 解除条件：在干净 x86-64 Linux 主机从公开发行物完成安装、健康、OIDC、对象存储、公开 Matrix 委派与双向联邦验证。
- 当前证据：[`specs/agent-room-foundation/task-40-validation.md`](task-40-validation.md)

### GNG-005 · 真实生产副本故障演练与 SLO 对账缺失

- 责任角色：运行与可观测性负责人
- 解除条件：在等价生产副本逐一演练控制平面、Matrix、对象存储、OIDC、Bridge 与联邦对端故障，验证分页告警、Runbook、恢复和原始指标一致。
- 当前证据：[`specs/agent-room-foundation/task-42-validation.md`](task-42-validation.md)

### GNG-006 · 离线根密钥仪式与受保护签名发行未执行

- 责任角色：发行维护者
- 解除条件：举行可审计的离线根密钥仪式，完成受保护候选与最终工作流、OCI/SBOM/签名复验，以及测试渠道升级、中断恢复和显式回滚。
- 当前证据：[`specs/agent-room-foundation/task-43-validation.md`](task-43-validation.md)

### GNG-007 · 仓库尚未公开且没有外部贡献者复现

- 责任角色：仓库维护者与外部贡献者
- 解除条件：在维护者批准后公开仓库，并由无开发缓存的外部贡献者按 README 完成开发启动；运营者在公网 Linux 按自托管指南部署且不手改数据库。
- 当前证据：[`specs/agent-room-foundation/task-44-validation.md`](task-44-validation.md)

## 5. 对外发布材料

- 版本说明：[`release/go-no-go/RELEASE-NOTES.md`](../../release/go-no-go/RELEASE-NOTES.md)
- 已知限制：[`docs/known-limitations.md`](../../docs/known-limitations.md)
- 数据与保留策略：[`release/closed-test/DATA-POLICY.md`](../../release/closed-test/DATA-POLICY.md)
- 安全联系方式：[`SECURITY.md`](../../SECURITY.md)

## 6. 重新评审规则

只有在每个开放阻断均有不可变证据、需求和门禁全部为 `pass`、`publicBetaEnabled` 明确改为 `true` 后，才允许把 JSON 决策改成 `go`。执行 `python tools/go_no_go.py assert-go` 必须返回成功；人工口头批准不能绕过该门。
