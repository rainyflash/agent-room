# 任务 10 Bridge 设备授权与撤销验证记录

> 验证日期：2026-08-23
>
> 结论：通过
>
> 对应任务：[实施计划 10](./tasks.md#m1内部纵向切片)
>
> 核心实现提交：`7fb2d27`、`8d62e57`、`9ac7378`、`1ed93a6`、`0ada9c7`、`65f7281`、`060fab6`、`296ed69`、`79b6bcd`、`a6ee601`

## 1. 架构边界

- 领域层定义设备公钥、信任状态和 Token Family 状态机，不依赖 HTTP、OIDC、SQLx、系统凭据库或 Bridge 进程。
- 应用层通过设备注册事务、设备会话仓储、证明 Nonce 仓储、签名验证器、设备仓储和撤销事务等端口编排用例。注册、刷新和撤销的业务规则不进入 Axum Handler。
- `identity-adapter` 承担 OIDC Device Authorization Grant、OIDC 断言校验、Ed25519 密钥和签名实现；`postgres-adapter` 承担原子持久化和并发锁；两者都只实现应用层端口。
- `bridge-core` 只负责编排设备授权和会话轮换。OS 安全存储、终端提示、Reqwest 和环境配置全部留在 `apps/bridge` 组合根。
- Windows Credential Manager、macOS Keychain 和 Linux Secret Service 由同一个 `DeviceCredentialVault` / `DeviceSigningIdentityStore` 边界隔离。平台安全存储不可用或内容损坏时明确失败，不降级到明文本地文件。

## 2. 设备授权与设备持有证明

- Bridge 通过 OIDC Discovery 获取设备授权端点、Token 端点和 JWKS，使用公开客户端发起 RFC 8628 Device Authorization Grant，并严格遵守 Provider 返回的轮询间隔、`slow_down`、到期和拒绝语义。
- 返回的 ID Token 必须通过签名、issuer、audience、expiration 和授权上下文校验；Bridge 客户端之外的 audience 会被拒绝。
- Bridge 首次运行在本机生成 Ed25519 密钥，私钥种子只写入 OS 安全存储；控制平面只持久化 32 字节公钥及算法标识。
- 设备注册签名覆盖一次性授权断言摘要、设备标签、平台和公钥，把“谁授权、哪台设备、哪把公钥”绑定为不可拆分的登记请求。
- 后续设备请求证明覆盖设备 ID、规范化方法、固定请求目标、正文摘要、签发时间、随机 Nonce 和当前凭据摘要。证明不能被搬到另一路由、另一正文或另一 Token 上使用。
- HTTP 客户端关闭自动重定向，生产地址只允许 HTTPS；明文 HTTP 仅允许严格回环地址。响应正文即使没有 `Content-Length` 也受 64 KiB 流式上限保护。

## 3. 会话轮换与撤销传播

- 控制平面只保存访问令牌和刷新令牌的 SHA-256 摘要；明文令牌仅在签发响应和 Bridge OS 安全存储中短暂存在。
- 每次设备重新登记都会原子撤销该设备原有活跃 Token Family，再创建新 Family、访问令牌和刷新令牌。
- 刷新事务先锁定刷新令牌、Family、设备和主体；并发刷新只能有一个请求完成轮换。旧刷新令牌再次出现时，整个 Family 和设备会被标记为泄露并撤销。
- Bridge 刷新前先把本地凭据原子标记为 `refresh_pending`。若网络中断导致服务端是否提交未知，Bridge 不会重放旧刷新令牌，必须重新授权，从而避免把正常轮换误判成令牌盗用。
- 撤销事务同时撤销设备、所有 Token Family、访问令牌和刷新令牌，并在同一事务写入安全 Outbox 事件。撤销后认证立即失败，传播事件可由后续消费者可靠投递。
- 设备列表使用当前活跃 Web 会话；撤销额外要求精确前端 Origin 和近期认证，避免普通会话或跨站请求静默踢掉设备。

## 4. 攻击、竞态与故障验收

| 验收项 | 结果 |
| --- | --- |
| 同一 OIDC 设备授权断言再次注册 | 一次性回执唯一约束拒绝重放 |
| 错误 audience 或无效 OIDC 断言 | 登记前拒绝 |
| 错误 Ed25519 持有证明 | 不写入设备 |
| 证明篡改方法、目标、正文、时间或凭据 | 签名或规范校验失败 |
| 同一证明 Nonce 再次使用 | 原子消费只允许第一次成功 |
| 两个请求并发刷新 | 只有一个完成轮换 |
| 旧刷新令牌重用 | 原子撤销设备与整个 Token Family，并写安全事件 |
| 刷新响应提交状态未知 | 本地保持待决，绝不重放旧令牌 |
| 设备撤销后使用原访问令牌 | 认证失败 |
| 非近期认证或错误 Origin 撤销 | HTTP 层拒绝，设备不变 |
| 控制面 2xx 返回畸形或超大正文 | 视为提交状态未知，不自动重试 |
| 非回环明文控制面地址 | Bridge 启动配置拒绝 |
| OS 安全存储损坏或不可用 | 明确失败，不回退到磁盘明文 |

敏感类型的 `Debug` 输出全部脱敏；控制平面日志不记录设备码、OIDC 断言、访问令牌、刷新令牌、签名或私钥。终端直接向当前用户展示设备验证码属于授权交互界面，不进入结构化日志。

## 5. 真实服务验收

- 使用全新隔离 Keycloak 26.7.2 导入项目 Realm，真实完成 Device Authorization Grant、用户批准、ID Token 校验、Ed25519 登记和控制面设备注册。
- 第二次启动直接从 Windows Credential Manager 恢复同一设备会话，没有再次要求 Device Grant。
- 隔离控制面把访问令牌时限缩短为 60 秒；Bridge 在刷新提前量内真实调用 `/auth/devices/refresh`，控制面返回 `200`，设备 ID 保持不变且凭据成功轮换。
- 真实 PostgreSQL 验证并发刷新、旧令牌重用、设备撤销、Token 失效和 Outbox 原子性。测试数据库每次新建并最终删除，不污染开发数据。
- 现有 Keycloak 开发卷的旧管理员凭据与当前生成配置不一致，因此验收使用独立、自动删除的容器，没有重置或篡改现有身份数据。
- 验收期间发现持久开发数据库尚未应用新增设备列；执行幂等迁移后恢复。正式入口 `just control-plane` 已把数据库迁移声明为启动前置，绕过该入口直接运行二进制不属于受支持的开发流程。

## 6. HTTP 接口

| 方法与路径 | 职责 |
| --- | --- |
| `POST /auth/devices/register` | 校验 OIDC Bearer 与设备持有证明，签发一次性设备凭据 |
| `POST /auth/devices/refresh` | 校验发送方约束证明并原子轮换设备凭据 |
| `GET /auth/devices` | 使用活跃 Web 会话列出当前主体设备 |
| `DELETE /auth/devices/{device_id}` | 要求同源和近期认证，撤销设备并发出传播事件 |

## 7. 平台与质量门禁

- Windows 上显式执行真实 OS 安全存储测试，完成签名种子和设备会话的写入、读取与清理。
- Linux 由现有 Ubuntu 全仓 CI 编译；无 Secret Service 时适配器安全失败。Windows 原生 CI 验证 Credential Manager 后端。macOS Keychain 代码路径保留，但只允许在维护者提供的自托管 Apple Silicon runner 上手动验收，不再租用 GitHub 托管 macOS runner。
- Windows 主机交叉检查 macOS 时已编译 `apple-native-keyring-store`，随后因本机没有 Apple C 编译器而停止；完整 macOS 原生链接必须由 `.github/workflows/macos-self-hosted.yml` 验证，缺少自托管 Mac 时明确保持未验证，不用 Windows 结果冒充。
- `just check` 全部通过：Rust/TypeScript 格式、Clippy `-D warnings`、类型检查、构建、测试、协议一致性、Secret 扫描和 GitHub Actions 固定版本检查均无失败。
- `python tools/database.py test` 的 12 个真实 PostgreSQL 测试全部通过，其中设备撤销与并发刷新 2 个场景覆盖任务 10 的原子性要求。
- `just coverage` 合并普通测试与真实 PostgreSQL 测试后，Rust 总行覆盖率为 77.75%，高于 60% 门禁。设备领域、设备应用层、Device Grant、PostgreSQL 设备适配器和设备 HTTP 层行覆盖率分别为 81.10%、78.70%、90.23%、88.74% 和 83.52%。TypeScript 四项覆盖率均为 100%。
- `cargo deny check` 的 advisories、bans、licenses 和 sources 通过；Node 官方审计端点未发现已知漏洞。
- OIDC 协议测试不再提交静态 RSA 私钥，测试启动时即时生成隔离密钥；Secret 扫描共检查 157 个文本文件并通过。

## 8. 实现依据

- OAuth 2.0 Device Authorization Grant 定义设备码、用户码、轮询间隔、`authorization_pending`、`slow_down` 和一次性使用语义：[RFC 8628](https://datatracker.ietf.org/doc/html/rfc8628)。
- OAuth 2.0 Security Best Current Practice 要求发送方约束、刷新令牌轮换或重放检测，并避免开放重定向和弱客户端认证：[RFC 9700](https://datatracker.ietf.org/doc/html/rfc9700)。
- Ed25519 的签名与公钥编码依据：[RFC 8032](https://datatracker.ietf.org/doc/html/rfc8032)。
- Keycloak 的设备授权端点和客户端能力由其官方管理指南与源码定义：[Keycloak Server Administration Guide](https://www.keycloak.org/docs/latest/server_admin/)、[Keycloak DeviceEndpoint](https://github.com/keycloak/keycloak/blob/main/services/src/main/java/org/keycloak/protocol/oidc/grants/device/endpoints/DeviceEndpoint.java)。
- 跨平台 OS 凭据库行为依据 `keyring` 官方文档：[keyring 4.1.6](https://docs.rs/keyring/4.1.6/keyring/)。
