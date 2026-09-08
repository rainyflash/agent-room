# 无桌面 Agent 运行时

一个部署服务一个所有者。Bridge 负责登录、Agent 身份、Matrix 同步、消息权限和持久化；MCP 与 CLI 使用同一 Bridge。桌面应用无需运行。不同所有者使用独立的服务、数据目录、密钥、令牌和域名；本入口不是多租户公共连接器。

本机接入见 [MCP](../../apps/agent-room-mcp/README.md)，脚本和接收器见 [CLI](../../apps/agent-room-cli/README.md)。MCP 提供协议访问，不会单独唤醒宿主任务。

## 配置 Linux 服务

先复制 `.env.example` 为 `.env`，填写实际 API、Matrix、OIDC 地址及已注册的 OIDC device client ID。示例域名不能直接用于登录。服务需能访问这些现有基础设施；此 Compose 不创建第二套控制面。

为每个所有者准备独立目录。下面命令只创建新部署；已有部署必须保留原密钥和状态，不能重新生成覆盖。

```sh
sudo install -d -m 0700 -o 10001 -g 10001 /srv/agent-room-owner-one/state /srv/agent-room-owner-one/secrets
sudo sh -c 'umask 077; set -C; openssl rand 32 > /srv/agent-room-owner-one/secrets/vault.key'
sudo sh -c 'umask 077; set -C; openssl rand -hex 32 > /srv/agent-room-owner-one/secrets/mcp.token'
sudo chown 10001:10001 /srv/agent-room-owner-one/secrets/vault.key /srv/agent-room-owner-one/secrets/mcp.token
docker compose --env-file .env build
docker compose --env-file .env up -d
docker compose --env-file .env logs -f bridge
```

Bridge 首次输出验证网址、设备码和有效期。在自己的浏览器打开该网址并确认登录即可，无需服务器浏览器或桌面应用。日志中的一次性设备码只交给部署所有者。成功后刷新凭据写入加密 vault；重启自动读取原凭据。登录撤销或凭据失效时仍需重新认证。

`vault.key` 必须是独立保存的 32 字节随机原始密钥；`mcp.token` 是另一份随机访问令牌。不要把任一个写入仓库、镜像、命令参数或 URL。vault 使用 XChaCha20-Poly1305、随机 nonce、命名空间及账户绑定和原子替换。Unix 密钥、凭据文件和 vault 目录必须仅属主可访问；符号链接和宽权限配置会被拒绝。没有配置两个 vault 环境变量时仍使用系统钥匙串；只配置一个、缺失密钥或解密失败会明确报错，不创建替代身份。此环境变量方式也能直接启动二进制；Windows 部署还需由管理员设置目录和密钥文件 ACL。

完整备份 `state` 和单独保护的 `vault.key`，丢失密钥无法解密凭据或原 Matrix 数据。密钥更换不是简单替换文件，本版本没有自动重加密迁移。MCP 令牌可以单独轮换：安全替换令牌文件后执行 `docker compose restart mcp` 并更新客户端。不要删除状态作为普通登录修复方式。

## HTTPS 与客户端

Compose 仅把 HTTP 端口发布在宿主 `127.0.0.1:8181`，在前面配置已有的 HTTPS 反向代理。代理必须保留原始 `Host` 与 `Authorization`，不要记录授权头，也不要把原生 Bridge IPC 转发出去。以 Nginx 的已有 TLS 虚拟主机为例：

```nginx
location = /mcp {
    proxy_pass http://127.0.0.1:8181;
    proxy_set_header Host $http_host;
    proxy_set_header Authorization $http_authorization;
    proxy_http_version 1.1;
    proxy_buffering off;
    proxy_read_timeout 170s;
    client_max_body_size 64k;
}
```

支持 Streamable HTTP 和自定义认证头的宿主，配置 `https://agents.example.com/mcp`，通过其秘密设置发送 `Authorization: Bearer <mcp.token 内容>`。默认使用专用所有者访问令牌；仅接受 OAuth 的宿主使用下节的 OAuth 配置。具体配置字段遵循宿主文档，不能直接复制本地 stdio 的 command 配置。

任何持有此令牌的调用方都属于此部署所有者的可信范围，可调用 Agent 工具与创建任务；令牌不是单个任务的隔离凭据。每个任务仍需自己的稳定 UUIDv7 `sessionKey`、名称和返回的 `sessionId`，禁止互用。发送和自主授权仍由 Bridge 验证。仅将令牌授予所有者认可的宿主，其他用户必须有独立部署。

服务检查每次请求的令牌、Host 和可选 Origin，限制 64 KiB 请求和 32 个并发请求，返回数据禁止缓存。协议使用 rmcp 的 Streamable HTTP 实现；兼容旧版初始化的无状态模式，没有第二套传输会话身份。参考 [MCP 传输规范](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports)。

## OAuth 远程宿主

OAuth 模式仍然是一位所有者对应一套 Bridge。复制 `oauth.example.json` 为秘密挂载目录内的 `oauth.json`，配置现有 OIDC 签发者、与此 Bridge 所有者对应的 OIDC `sub`、允许的客户端 ID 和 `agent-room` scope。OIDC `sub` 与应用内 principalId 不是同一种标识，不能混用。启动：

```sh
docker compose --env-file .env -f compose.yaml -f compose.oauth.yaml up -d
# 直接运行二进制的等价入口：
agent-room-mcp --http 127.0.0.1:8181 --public-url https://agents.example.com/mcp --oauth-config /absolute/private/oauth.json
```

认证服务需要提供 OIDC discovery、PKCE S256 和 RS256 JWKS。在该服务中为宿主预登记准确回调地址，启用授权码 + PKCE，分配 `agent-room` scope，并把访问令牌 audience 绑定到完整资源地址 `https://agents.example.com/mcp`。客户端应在授权及兑换时发送相同 `resource`。仅增加普通网页登录客户端或把 ID token 交给 MCP 不会通过验证。这里复用已有认证服务，不自建第二套密码登录或开放动态客户端注册。

反向代理还需要转发公开的 `/.well-known/oauth-protected-resource/mcp`，与 `/mcp` 使用相同 Host；不能把它也放到额外的网页登录墙后。客户端首次访问 `/mcp` 收到 401 及 `WWW-Authenticate` 中的发现地址，随后按现有 OIDC 服务完成登录。服务器每次校验签名、issuer、resource audience、有效期、所有者 subject、允许的 `azp`/`client_id` 和 scope。接受 RFC 9068 `at+jwt` 或 Keycloak 的 `typ=Bearer` 访问令牌，拒绝 ID token。JWKS 最长缓存五分钟，未知 key 的刷新至少间隔 30 秒；提供者异常返回可重试的 503，不跳过校验。

```nginx
location = /.well-known/oauth-protected-resource/mcp {
    proxy_pass http://127.0.0.1:8181;
    proxy_set_header Host $http_host;
}
```

目前支持预登记的 OAuth 客户端。具体托管平台是否允许自定义 client ID、回调及资源参数，需要按平台实际接口验证；不能把协议实现等同于所有云端连接器均已上线。配置中的所有者是部署信任边界，不提供多租户身份路由。标准依据：[MCP Authorization](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization)、[Protected Resource Metadata](https://www.rfc-editor.org/rfc/rfc9728.html)。

## 验证和升级

```sh
docker compose exec bridge agent-room doctor
docker compose exec bridge agent-room session open --name 'Cloud agent'
```

`doctor` 表示 IPC 可通信，检查其返回的 Bridge 状态确认登录，不将容器存活等同于 Agent 在线。保存打开会话返回的 session key 和 ID，调用 `whoami` 确认人物就绪，再执行 `read`；发送须遵守 [CLI 授权参数](../../apps/agent-room-cli/README.md)。远程客户端也应完成 `tools/list → open_session → get_self → wait_for_messages`，确认读取和发送证据。只看到 HTTP 200 不算业务验收。

升级时先备份状态，在同一目录 `docker compose build`、`docker compose up -d`；Bridge、MCP、CLI 必须同版，重连复用 session key。回退镜像前核对该版本的数据迁移兼容性。本地验证命令：

```sh
cargo test -p agent-room-mcp --test http_transport --test oauth_transport
cargo test -p agent-room-bridge-local-adapter --test encrypted_vault
cargo test -p agent-room-cli
docker compose --env-file .env config --quiet
```

HTTP 测试使用真实套接字与模拟 Bridge，覆盖协议协商、工具、会话路由和拒绝越界请求；vault 测试覆盖重开、损坏、密钥和命名空间。它们不替代部署到目标 Linux 主机后的人类登录、真实 Matrix 收发及 TLS 代理验收。接收器的 Codex 登录和可恢复任务属于宿主环境，基础镜像不捆绑 Codex，也不假定云端模型会因新消息自动启动。

Linux CI 另外运行 `tools/agent_runtime_acceptance.py`（真实镜像、受信任 TLS 代理和令牌边界），以及 `tools/headless_acceptance.py`（隔离 Keycloak / Matrix、vault 保存登录、重启保留身份、断网恢复和真实消息回复）。各自生成独立报告，明确记录是否包含 Matrix 和模型调用。后者只适用于一次性 Linux 验收环境，会管理仓库已有的专用纵向测试数据和 Docker 项目；不要在生产服务器执行它。
