# 任务 37 验证记录：双 Homeserver 联邦环境

## 1. 结论

任务 37 的实现与独立 CI 验收已经完成。测试拓扑不是同一进程内的伪联邦：两个 Synapse 分别使用独立域名、PostgreSQL、签名密钥和控制域，经 Caddy TLS/委派后执行真实 Matrix federation。任务 36 的封闭测试发布门仍是任务依赖；在该门变绿前，本记录不把 M3 当作可公开发布许可。

## 2. 可重复拓扑

`infra/federation/compose.yaml` 启动两套隔离的 PostgreSQL 与 Synapse，并由反向代理提供客户端入口、`/.well-known/matrix/server`、`/.well-known/matrix/client` 和联邦入口。`tools/federation.py` 负责生成一次性域名、密钥、用户与房间夹具，避免测试依赖开发者长期服务或共享凭据。

自动预检覆盖：

- 两端 TLS、委派、`/_matrix/federation/v1/version` 与签名密钥可达；
- 两个 Homeserver 的 server name、数据库、签名材料和控制域互不复用；
- 联邦目标只信任本次夹具域名，不把回环地址或内部容器地址写入公开结果。

## 3. 跨服务证据

真实验收逐项记录三个不同事实，避免把“本地 API 返回成功”冒充“对端已经收到”：

1. 发送端本地接受事件并返回稳定事件 ID；
2. 接收端同步时间线观察到同一事件 ID；
3. 发送端收到由接收方账号产生的已读回执。

同一拓扑还验证跨服务房间加入、消息预览与按需正文、状态续租、封禁/解封、设备 E2EE 会话、对端停机期间本地继续工作，以及恢复后的有界回填。停机窗口内的消息不会因为重试生成重复事件。

## 4. 自动化门禁

```text
python tools/federation.py bootstrap
  双 Homeserver 真实联邦场景通过
  报告：artifacts/federation/task-37-report.json

GitHub Actions：M3 联邦验收
  运行 32904790173：通过
  运行 32905649706：通过
```

成功运行同时上传拓扑诊断、测试报告和容器日志；失败时也会收集诊断，不依赖临时终端输出追责。

## 5. 提交

- `d3aa1e5`：建立双 Homeserver 验收夹具。
- `e915faf`：收紧可信联邦恢复并接入独立 CI。

## 6. 下一步

任务 38 在此真实入口上施加版本协商、对端/房间/用户治理、重放抑制和状态冲突隔离。任务 39 使用同一拓扑测量断网回填容量，而不是另造一套与生产行为不同的模拟服务器。
