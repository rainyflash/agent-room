# 任务 16.6：浏览器多设备与跨账户闭环证据

## 结论

三个相互隔离的真实 Chromium 上下文已经完成云端优先闭环。测试期间两套 Bridge 进程都被主动停止；Web 仍可仅凭 Agent Room 账户会话读取 Control Plane 与 Matrix，证明它不再是本机 Bridge 的遥控器。

## 执行入口

```text
python -m py_compile tools/vertical.py
python -m unittest tools.tests.test_vertical
python tools/vertical.py bootstrap
```

执行时间：2026-08-30。

隔离 Compose 项目：`agent-room-vertical-24`。每次执行都会重建 PostgreSQL、Keycloak、Synapse、对象存储和内容扫描数据，并在退出后删除隔离数据卷。

## 通过场景

| 场景 | 真实证据 |
| --- | --- |
| 同账户双设备 | 两个完全独立的 BrowserContext 使用同一 OIDC/Matrix 用户登录，得到相同 Matrix User ID 与两个不同 Matrix Device ID。 |
| 无 Bridge 云端工作区 | 两套 Bridge 均停机时，两个账户仍能打开 `/workspace`，直接读取 Agent、设备和实例云端事实；页面不存在 Tauri 或桌面 Runtime 表面。 |
| 第二真实账户 | 独立 OIDC 用户 `Local Collaborator` 建立自己的 Control Plane principal 与 Matrix 身份，不与开发账户复用 Cookie、Storage 或 Matrix 凭据。 |
| 共享 Agent 权限 | 所有者通过最近认证会话把第二账户授予共享 Agent 的 `operator` 角色；第二账户随后在工作区读到同一个 Agent。 |
| 同一公共大厅 | 三个会话分别执行真实大厅入口流程，最后解析到同一个 Matrix Room ID。 |
| 跨账户消息 | 第二账户通过消息发送器上传正文、发送人类 Matrix 事件并绑定内容；所有者两个会话都观察到消息，其中一个显式打开并校验完整正文。 |
| UI 定向交接 | 第二账户从消息详情打开经摘要校验的正文，点击“交给 Agent”，显式选择离线目标实例并确认一次性交接。 |
| 离线排队与恢复 | 创建交接时目标 Bridge 处于停止状态；目标 Bridge 恢复后在线 generation 增至 4，并领取排队交接。 |
| MCP 一次性消费 | 目标 `agent-room-mcp` 明确消费交接，验证人类 principal、来源房间、事件、消息、正文引用和正文内容，数据库进入 `consumed` 终态。 |
| 日志脱敏 | 反向扫描 Control Plane、Web、两套 Bridge 和五套 MCP 共 12 份日志，未发现密码、JWT、设备码或已知凭据。 |

## 本次证据标识

以下标识只用于关联本次隔离验收，不包含认证凭据：

```text
共享 Agent：01a0522e-ea98-7f40-a38b-c7ca229dfc38
目标实例：01a0522f-ab39-7782-b283-79758b4ff41a
第二账户 principal：01a05230-b587-74e1-855f-5f31e3484c6e
跨账户交接：01a05230-f65e-77ff-ba69-956dd1072d2b
Matrix 房间：!uQYBhjVADmwVJlVJin:matrix.agent-room.localhost
```

机器可读结果保存在被 Git 忽略的 `.local/vertical/product-closure.json`；失败截图、Trace 与服务日志位于 `artifacts/browser/cloud-first-product-closure/`。运行时证据不提交到仓库，避免把临时账户会话或本机路径固化为产品资产。

## 配套证据

真实 Windows Tauri 进程的 Bridge 离线降级由独立门禁完成，证据见[任务 16.6.3 桌面验证](./task-16-6-desktop-validation.md)。浏览器与桌面两份证据共同构成任务 16.6 的完成条件。
