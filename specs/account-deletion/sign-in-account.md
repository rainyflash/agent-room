# 删除账户时删掉登录账户

2026-10-08 起草隐私说明页（#341）时发现：删除账户会停用并擦除聊天账户、匿名化控制面里的记录，但 Keycloak 里的登录账户一直留着，邮箱、昵称和密码散列都在那里。隐私说明页要写“删除账户会删掉登录账户”，这里记下怎么补。

## 做法

- 删除任务在“停用外部账户”那一步，先删 Keycloak 用户，再停用并擦除聊天账户，最后匿名化本地记录（`AccountDeletionWorker`）。先删登录账户，之后就登录不了。
- 控制面用一个新的 Keycloak 客户端 `agent-room-account-admin` 拿令牌。它只开服务账号：不能登录、不能用密码换令牌、没有回调地址；服务账号只有 realm-management 的 `manage-users`。令牌和管理接口都走内部地址 `http://identity:8080`。
- 只删签发方是本部署的账户（和 `AGENT_ROOM_OIDC_ISSUER_URL` 一样）。网络 Agent 的主体签发方是 `urn:agent-room:network-agent`，没有登录账户，跳过。
- 删除任务领取时从主体记录带出签发方和用户号（`principal.oidc_issuer`、`oidc_subject`），匿名化之前它们都还在。
- 用户已经不在（404）算成功。Keycloak 暂时不可用、拒绝请求，都按原来的退避重试，失败码是 `identity.*`。任务重试、备份恢复后重放（`account-deletion-replay.sh` 重新排队）都会再删一次，删过的回 404，没关系。

## 部署

- 新 Secret `keycloak_account_admin_client_secret`，`SecretStore.initialize` 发现没有会自动生成。
- 每次部署的身份同步（`keycloak-registration-reconcile.py`）建好这个客户端、同步密钥、给服务账号授权；新装时导入的领域文件里也带着它。
- 控制面新配置：`AGENT_ROOM_KEYCLOAK_INTERNAL_URL`、`AGENT_ROOM_KEYCLOAK_ACCOUNT_ADMIN_CLIENT_SECRET`（或 `_FILE`），可选 `AGENT_ROOM_KEYCLOAK_ACCOUNT_ADMIN_CLIENT_ID`（默认 `agent-room-account-admin`）。
- 本地和 CI 的 Keycloak 由 `tools/dev-infra.ps1` 同步同一个客户端，`.env.local` 缺 `KEYCLOAK_ACCOUNT_ADMIN_CLIENT_SECRET` 时 `prepare` 会补上。
- 改到了 `render.py`、身份同步脚本和 `crates/identity-adapter/`，发版时实机验收要维护者批准一次设备码。

## 取舍

- `manage-users` 能改任何用户，包括重设密码；Keycloak 没有只能删用户的角色。控制面本来就能以应用服务身份冒充任何聊天账户，这不算多给了它什么。
- 生产上至今没有删过账户（2026-10-08 只读查过：没有删除任务，2 个人的账户、21 个网络 Agent），不用补删以前留下的登录账户。

## 状态

- 2026-10-08：设计和实现在同一个 PR。
