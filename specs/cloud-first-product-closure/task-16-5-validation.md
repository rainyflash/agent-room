# 任务 16.5：云端交接纵向闭环证据

## 结论

云端交接闭环已在全新隔离环境中完成真实纵向验证。验证没有使用 Mock HTTP、内存数据库或同一份设备配置冒充多设备，而是启动真实 PostgreSQL、Synapse、OIDC、对象存储、内容扫描、Control Plane、Web、两套隔离 Bridge 和两套 MCP 进程。

## 执行入口

```text
python -m py_compile tools/vertical.py
python tools/vertical.py bootstrap
```

执行时间：2026-08-30。

隔离 Compose 项目：`agent-room-vertical-24`。脚本在验收结束后删除隔离容器、网络与数据卷，并恢复开发环境，不复用上一次运行的数据库和 Bridge 收件箱。

## 通过场景

| 场景 | 真实证据 |
| --- | --- |
| 浏览器用户创建交接 | Playwright 使用真实 OIDC 会话进入大厅，枚举目标实例并调用云端交接 API。 |
| 幂等创建 | 同一个 `Idempotency-Key` 首次返回 `201 created=true`，重放返回 `200 created=false`，交接 ID 不变。 |
| 目标离线排队 | 创建第一笔交接前主动停止目标 Bridge；云端记录保持排队，发送端 MCP 无法读取。 |
| 同设备身份恢复 | 目标 Bridge 使用原持久设备配置重启，Agent 在线 generation 从 2 增至 3，随后领取排队交接。 |
| 目标隔离 | 发送端实例的 `agent_room_list_handoffs` 看不到目标实例交接。 |
| 延迟正文读取 | Bridge 领取阶段只落盘元数据；目标 MCP 明确消费时才打开内容并校验摘要、长度、媒体类型和原始正文。 |
| 一次性消费 | 第一笔交接进入数据库 `consumed` 终态；再次消费只返回不可重试的终态错误，不返回正文。 |
| 显式拒绝 | 第二笔交接由目标 MCP 拒绝并进入数据库 `declined` 终态；再次处理安全失败。 |
| 人类发起者投影 | MCP 响应保持 `HumanActor` 和真实 principal ID，没有伪装成 Agent。 |
| 日志脱敏 | 扫描 Control Plane、两套 Bridge 与四套 MCP 共 11 份日志，没有发现会话、设备密钥或已知凭据。 |

## 本次证据标识

以下标识只用于关联本次测试数据库与日志，不包含认证凭据：

```text
目标实例：01a0521e-fc52-78b0-a77a-b196bda938ca
发送实例：01a0521e-b6e3-7ca2-bd91-02eae63c3adb
已消费交接：01a0521f-6216-7d4d-88e0-984d0fe026ac
已拒绝交接：01a0521f-c419-7c62-a15e-d8a7ce721287
Matrix 房间：!NrviJEEDEoQqEvfIap:matrix.agent-room.localhost
```

完整机器可读结果保存在被 Git 忽略的 `.local/vertical/bootstrap.json`；服务日志保存在 `artifacts/browser/task-24/services/`。这些运行时证据不提交到仓库，避免把临时环境信息当成产品源码。

## 失败后修正

第一次执行在“二次消费”的验收断言处失败：本机收件箱在云端回执成功后会物理删除元数据，因此再次消费返回 `bridge.handoff_not_found`，而不是固定返回 `bridge.handoff_already_resolved`。这不是业务缺陷，而是数据最小化策略的结果。

验收已改为只接受两个不可重试的安全终态：`bridge.handoff_already_resolved` 或 `bridge.handoff_not_found`；数据库终态仍被独立断言为 `consumed` 或 `declined`。随后从全新数据卷重新执行，完整流程退出码为 0。
