# 任务 9 OIDC 登录与主体投影验证记录

> 验证日期：2026-08-23
>
> 结论：通过
>
> 对应任务：[实施计划 9](./tasks.md#m1内部纵向切片)
>
> 核心实现提交：`f329fe0`、`eb5a806`、`38e78a8`、`32ce5c0`、`ca1e391`

## 1. 架构边界

- 应用层只依赖 `OidcGateway`、登录尝试仓储、登录完成事务、会话仓储和主体暂停事务等端口，不导入 HTTP、SQLx 或具体 OIDC SDK。
- `identity-adapter` 独立承担 OIDC Discovery、授权 URL、授权码兑换和 ID Token 验证；协议实现可替换，不污染领域模型。
- PostgreSQL 适配器把“幂等创建或读取 Principal + 建立 Web Session”放在一个事务中。唯一键竞争后重新读取权威主体，不靠应用层先查后写制造竞态。
- Axum 认证功能只负责 HTTP 参数、Cookie、Origin、缓存头和错误映射，业务状态机留在应用层。
- 控制平面组合根注入真实 OIDC 与 PostgreSQL 适配器；配置缺失或非法时启动立即失败。

## 2. OIDC 协议与身份校验

- 使用 Authorization Code + S256 PKCE，每次登录生成独立 `state`、`nonce`、PKCE verifier 和浏览器绑定秘密。
- OIDC Discovery 在适配器构造阶段完成并缓存；HTTP 客户端禁止自动重定向，避免发现端点或 Token 端点借 3xx 改变信任边界。
- ID Token 验证签名、issuer、audience、expiration、nonce、`auth_time` 与 `at_hash`。HMAC 算法使用客户端秘密验证 `at_hash`，非对称算法使用 JWKS 中的签名公钥。
- 登录尝试同时绑定浏览器秘密摘要和状态摘要，并通过原子消费抵抗 Login CSRF、重放和并发双击。
- 首次登录以 `(oidc_issuer, oidc_subject)` 幂等创建 Agent Room Principal，并生成稳定、唯一的 Matrix User 映射。

## 3. Web 会话与资料最小化

- 登录中间态使用 `__Host-agent-room-login`，登录完成后轮换为 `__Host-agent-room-session`；两者均设置 `Secure`、`HttpOnly`、`SameSite=Lax`、`Path=/` 和有限 `Max-Age`。
- 数据库只保存 Cookie 秘密与 OIDC 状态的 SHA-256 摘要，不保存 OIDC Access Token；所有认证响应设置 `Cache-Control: no-store` 和 `Referrer-Policy: no-referrer`。
- `returnTo` 只接受本站绝对路径，最终跳转始终拼接固定前端 Origin；`//evil.example`、反斜杠、控制字符和跨源 URL 在调用用例前即被拒绝。
- 注销要求请求 `Origin` 与配置的前端 Origin 精确相等，随后服务端撤销会话并删除 Cookie。
- 主体暂停立即阻止所有已有会话；普通会话与近期认证采用不同时间窗口。OIDC 故障不强制注销仍有效的本地会话。
- 第三方显示名称与语言逐字段、默认关闭导入；未明确同意时使用 Agent Room 自有默认资料。

## 4. 攻击与并发验收

| 验收项 | 结果 |
| --- | --- |
| 错误浏览器状态或缺失绑定 Cookie | 在 Token 兑换前拒绝，正确状态仍可继续 |
| 错误 issuer、audience、nonce、`at_hash` | ID Token 拒绝 |
| 过期 Token、缺失/过旧/未来 `auth_time` | 会话不创建 |
| 错误 PKCE verifier | Provider 拒绝授权码兑换 |
| 并发相同 subject 首次登录 | 只创建一个 Principal，各自建立独立会话 |
| 会话插入冲突 | 新 Principal 与会话整体回滚 |
| 登录尝试重放 | 第一次消费成功，后续消费失败 |
| 开放跳转 | HTTP 层在进入用例前拒绝 |
| 注销 CSRF | 非精确 Origin 返回 `403`，会话不撤销 |
| 主体暂停 | 已签发会话立即失效 |
| 资料导入未同意 | 第三方名称和语言不进入主体投影 |

本地真实 Keycloak 已完成 Discovery 和授权起点验证：控制平面返回 `303`，授权地址包含独立 `state`、`nonce`、S256 challenge 与 `max_age`，登录 Cookie 安全属性完整。现有旧开发卷因管理员凭据漂移无法无损同步新增回调地址；启动脚本会警告并保留数据，全新环境导入配置不受影响。

## 5. HTTP 接口

| 方法与路径 | 职责 |
| --- | --- |
| `GET /auth/oidc/start` | 校验回跳路径和资料导入同意，创建登录尝试并跳转 IdP |
| `GET /auth/oidc/callback` | 校验浏览器绑定、状态和 Token，轮换安全会话 Cookie |
| `GET /auth/session` | 从 HttpOnly Cookie 返回当前活跃主体投影 |
| `POST /auth/logout` | 校验前端 Origin，撤销服务端会话并清除 Cookie |

## 6. 质量门禁

- `just check` 全部通过：Rust/TypeScript 格式、Clippy `-D warnings`、类型检查、构建、测试、协议生成一致性、密钥扫描和 GitHub Actions 固定版本检查均无失败。
- `just database-integration` 的 10 个真实 PostgreSQL 测试全部通过，其中任务 9 新增 3 个原子性、并发与一次性消费场景。
- `just control-plane-integration` 的真实依赖及逐层断连测试通过。
- `just coverage` 合并普通测试与真实 PostgreSQL 测试后，Rust 行覆盖率为 78.77%，高于 60% 门禁；认证应用层、OIDC 适配器、PostgreSQL 认证适配器和 HTTP 认证功能的行覆盖率分别为 88.22%、85.84%、87.79% 和 91.49%。TypeScript 四项覆盖率均为 100%。
- `cargo deny check` 的 bans、licenses 和 sources 通过。`RUSTSEC-2023-0071` 仅影响 `rsa` 私钥运算；生产路径经 `openidconnect` 只使用公开 JWKS 验签，隔离回环协议测试使用进程内临时 RSA 密钥签名。上游尚无修复版本，因此 `deny.toml` 记录了带理由的临时精确例外，升级后必须移除。
- 使用 npm 官方审计端点执行 `pnpm audit --audit-level high`，未发现已知漏洞。
- Windows MSVC 的本地化 `linker_messages` 仍是无害提示，不影响编译或测试结论。

## 7. 实现依据

- OpenID Connect Core 要求客户端验证 issuer、audience、签名、expiration 和 nonce，并定义 `at_hash` 校验：[OpenID Connect Core 1.0](https://openid.net/specs/openid-connect-core-1_0.html)。
- OAuth 2.0 for Browser-Based Apps 要求授权码流程使用 PKCE，并说明浏览器应用的重定向与状态保护：[OAuth 2.0 for Browser-Based Apps](https://datatracker.ietf.org/doc/html/draft-ietf-oauth-browser-based-apps)。
- Cookie 的 `__Host-` 前缀要求 `Secure`、`Path=/` 且不能设置 `Domain`：[MDN Secure cookie configuration](https://developer.mozilla.org/en-US/docs/Web/Security/Practical_implementation_guides/Cookies)。
- RustSec 明确说明 `RUSTSEC-2023-0071` 泄漏的是 RSA 私钥信息且当前没有修复版本：[RustSec RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html)。
