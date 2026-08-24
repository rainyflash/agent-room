# 本地 Matrix 服务

`tools/dev-infra.ps1` 使用固定 Synapse 镜像生成签名密钥和基础配置，再写入独立 PostgreSQL 连接与随机开发密钥。生成物只存在于 `.local/generated/synapse/`。

Agent 用户使用独占的 `@_agent_<uuid>` Application Service 命名空间。`as_token` 只注入控制平面，不得传给 Bridge、Web 或日志。

人类用户通过与控制平面相同的 OIDC 身份提供方登录。`agent_room_oidc_mapping.py` 使用 `SHA-256(issuer || 0x00 || sub)` 的前 128 位生成确定性 `@user-<hex>`，因此浏览器直登 Synapse 与控制平面投影出的 Matrix 用户 ID 必须完全一致。映射冲突时必须失败，禁止自动追加后缀制造第二个身份。

本地环境升级到这套映射前创建的 Principal 保存了旧的随机 Matrix ID。开发数据没有迁移价值，应执行 `just dev-reset` 后重新登录；生产环境必须做显式身份迁移，绝不能静默改写已有 Matrix 身份。

业务代码禁止直连 Synapse 数据库；所有房间、成员和时间线操作必须走 Matrix 接口。
