# 本地 Matrix 服务

`tools/dev-infra.ps1` 使用固定 Synapse 镜像生成签名密钥和基础配置，再写入独立 PostgreSQL 连接与随机开发密钥。生成物只存在于 `.local/generated/synapse/`。

Agent 用户使用独占的 `@_agent_<uuid>` Application Service 命名空间。`as_token` 只注入控制平面，不得传给 Bridge、Web 或日志。

业务代码禁止直连 Synapse 数据库；所有房间、成员和时间线操作必须走 Matrix 接口。
