# 任务 41 验证记录：备份、恢复与数据删除

## 1. 结论

任务 41 已完成。生产备份、15 分钟 RPO 调度、隔离 PITR 恢复、Homeserver 身份恢复、对象摘要校验、投影重建、保留策略以及账户导出/删除 Saga 均已有可重复执行的实现与证据。

本次隔离恢复实测 RTO 为 **30.68 秒**，低于核心单区 4 小时目标；PostgreSQL `archive_timeout` 与 systemd 调度均以 **900 秒**为硬上限。原始证据保存在 [`evidence/task-41-backup-restore.json`](./evidence/task-41-backup-restore.json)。

## 2. 备份与隔离恢复

生产工具自动备份并校验：

- Agent Room、Synapse、Keycloak 三个 PostgreSQL 数据库；
- 嵌入式 PostgreSQL base backup、WAL 与命名恢复点；
- Synapse 原 signing key 和 Keycloak Realm 配置；
- 私有对象桶及逐对象 SHA-256；
- 已完成账户删除的最小墓碑快照；
- 只读 manifest、完成标记和备份目录权限。

恢复演练不会覆盖生产目录。它创建隔离 PostgreSQL 容器，按命名恢复点重放 WAL，验证三个数据库，恢复原 signing key，校验对象摘要，重放删除墓碑，并从 Matrix 权威数据重建可丢失投影。只有全部断言通过才发布证据。

备份计划由 `agent-room-backup.timer` 自动安装和验证；失败不会伪装成成功快照，半成品始终停留在 `.partial-*` 目录。

## 3. 保留与回收

- Synapse 公共默认消息保留为 30 天，房间可在 1 天至 3,650 天之间配置；
- Agent Room 房间目录保存 `retention_days`，建房时写入 Matrix `m.room.retention`；
- 上下文交接包使用领域层过期时间，过期后不再授权读取；
- `uploading` 卡死对象、到期对象、未绑定事件对象与 `redacted/orphaned` 对象进入幂等回收；
- 生产备份按运营配置轮换。删除完成不承诺从尚未到期的加密备份中立即物理移除；恢复后必须重新执行删除账本与内容回收，且恢复环境不得面向用户开放。

每次备份都会从权威数据库导出已完成删除的最小墓碑，并原子合并到备份仓库根目录的单调账本 `ACCOUNT_DELETION_LEDGER.json`。普通备份轮换不会删除该账本；既有条目不能被改写，也不能为同一主体建立第二条记录。恢复较旧快照时，账本中仍存活的主体会先被置为 `deleting` 并重新排入删除 Saga。该账本本身仍属于敏感恢复材料，必须随备份仓库加密和限制访问。

## 4. 账户导出与删除协议

控制平面提供：

- `GET /account/export`：导出本地主体、设备、Agent、房间、内容引用和治理记录；
- `DELETE /account`：要求近期认证、精确输入 `DELETE`、确认联邦残留，并使用 UUIDv7 幂等键；
- `GET /account/deletion`：使用不记入日志的删除回执查询 Saga 进度。

删除回执由独立 256 位以上 HMAC 密钥和幂等任务 ID 确定性派生。数据库只保存 SHA-256 摘要；即使首次 `202` 响应丢失且会话已经撤销，同一请求仍可取回完全相同的回执。不同任务 ID 不能接管既有删除任务。

删除 Worker 使用数据库租约和 `SKIP LOCKED` 保证多副本至多一个执行者。处理顺序为：

1. 原子把主体置为 `deleting` 并撤销 Web、设备、Agent 实例和自动化凭据；
2. 使用仅存在于后端的 Synapse 管理令牌执行 `erase=true` 停用；
3. 清空 SSO external IDs，并分页删除该用户在本 Homeserver 的媒体；
4. 匿名化本地资料、归档独占私人房间、退役独占 Agent、撤销成员关系与正文引用；
5. 将对象标记为 `redacted/orphaned` 交给异步物理回收，并保留不含正文的最小审计墓碑。

Matrix 外部副作用失败时任务指数退避；只有 Matrix 本地清理成功后才进入本地匿名化。生产安装器幂等供应专用 Synapse 管理员，令牌写入权限受限 Secret 文件；Caddy 对公网 `/_synapse/admin/*` 固定返回 404。

## 5. 准确的数据边界

[Synapse 官方管理 API](https://element-hq.github.io/synapse/latest/admin_api/user_admin_api.html) 明确说明，`erase=true` 仍不会删除已经发送/接收的消息，也不会控制远端 Homeserver 的副本。因此产品必须显示以下事实，不能写成“全球彻底删除”：

- 已经在房间中的接收者仍可能看到历史事件；
- 联邦对端、接收者设备和其备份由各自运营者控制；
- 尚在保留期内的加密备份只用于灾难恢复，并在轮换到期后物理消失；
- Matrix timeline 不在本地结构化导出中，用户应在发起删除前通过 Matrix 客户端导出；
- 审计墓碑、事件 ID 和时间戳可为安全与一致性保留，但不保存消息正文或显示资料。

## 6. 自动门禁

```text
python tools/database.py test
  真实 PostgreSQL 全量测试通过；账户导出/删除事务、最小权限和 45 张表迁移通过

cargo test --locked --package agent-room-application --test account_lifecycle_flow
  5 通过：近期认证、联邦确认、回执重放、失败退避、完整删除顺序

cargo test --locked --package agent-room-matrix-provisioning-adapter accounts::tests
  3 通过：本地擦除/SSO/媒体、幂等缺失、远端拒绝

cargo test --locked --package agent-room-identity-adapter
  HMAC 回执稳定派生、跨任务隔离和弱密钥拒绝通过

python -m unittest tools.tests.test_local_runtime tools.tests.test_prodops
  23 通过：本地 Secret 门禁、生产管理员供应和公网 Admin API 封锁

python -m unittest tools.tests.test_backup tools.tests.test_restore
  8 通过：备份恢复契约、单调删除账本、轮换保留、隔离恢复重放入口

python tools/production.py validate --config infra/production/deployment.example.json ...
  生产 Compose 真实解析通过
```

## 7. 仍需运营遵守的约束

任务 41 的工程门禁已完成，但任务 40 的公网 Linux 验收仍是独立 No-Go 项。运营者必须执行定时备份验证与季度恢复演练；不能因为本次 30.68 秒实验结果就假设任意真实数据规模都具有相同 RTO。
