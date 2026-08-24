# 任务 14 Agent Card 与通用 A2A 资料适配验证记录

> 验证日期：2026-08-24
>
> 结论：通过
>
> 对应任务：[实施计划 14](./tasks.md#m1内部纵向切片)
>
> 核心实现提交：`7542c95`、`ce2c8a4`、`18a4ddc`、`6070b63`、`04ebce9`、`2012b02`

## 1. 架构边界

- `domain::agent_cards` 只表达规范化资料、端点验证、摘要、快照和过期语义，不依赖 HTTP、JWS、SQLx 或 Axum。
- `application::agent_cards` 负责设备有效性、Owner/Operator 权限、抓取编排、变化分类和缓存时限；Viewer 在触发任何外部请求前被拒绝。
- `a2a-adapter` 独立承担不受信任网络文档、A2A 1.0 wire schema、JCS 规范化和 JWS/JWKS 验证。控制平面只依赖 `AgentCardSource` 抽象。
- `postgres-adapter` 保存版本化的安全投影，不保存原始 Card、签名、技能示例、OAuth 流程细节、私有扩展参数或凭据。
- Axum 路由只处理设备签名认证、DTO 解析和公开响应投影，不复制权限判断或资料规范化规则。

## 2. 网络与签名边界

- 来源和所有兼容端点必须是无用户名、无密码、无片段的绝对 HTTPS URL。
- DNS 结果在连接前整体校验；任何回环、RFC1918、链路本地、云元数据、运营商 NAT、文档网段、保留或混合公私地址都会拒绝。批准的地址被固定到单次 Reqwest 客户端，TLS 主机名不变，并在响应后复核实际远端 IP。
- HTTP 客户端禁用系统代理与重定向，限制连接/请求超时，只接受 `application/json` 或 `application/a2a+json`，同时执行 `Content-Length` 预检和 64 KiB 流式硬上限。
- 签名使用 A2A 1.0 规定的 JCS 规范载荷和 JWS。只允许非对称算法；拒绝 HMAC、未知关键头、`b64=false`、受保护/未保护头冲突、重复 `kid`、错误 key use/ops/alg 和跨来源 JWKS。
- 多签名用于密钥轮换：至少一个合法签名即可通过；只要 Card 声明了签名但没有任何签名可验证，刷新就失败，不降级为未验证。

## 3. 规范化、缓存与资料映射

- 官方 A2A 1.0 结构 Fixture 映射为稳定名称、说明、提供方、版本、兼容端点、协议绑定、端点验证状态、能力、认证方案种类、媒体类型和技能摘要。
- 不支持的接口版本与未知必需扩展明确失败；不会为了“兼容”静默删除安全语义。
- 示例提示词不进入规范化摘要或持久化投影，防止把远端提示注入材料升级为受信资料。名称/说明变化与能力面变化分别报告 `profile_changed` 和 `capability_surface_changed`。
- 上游缓存 `max-age` 被限制在 1 秒至 1 小时，每份快照都有确定过期时间。读取时按当前时间派生 `expired`，不修改历史事实。
- PostgreSQL 用每 Agent 事务级 advisory lock 串行化写入，并物理保留最近 10 份且不超过 90 天的历史；并发写入不会绕过上限。

## 4. 控制面接口

- `POST /agents/{agentId}/agent-card/refresh` 只接受 16 KiB 内、字段封闭的 JSON；设备 Bearer 与 Ed25519 持有证明覆盖大写方法、精确路径和原始 UTF-8 正文摘要。
- 路由调用应用用例前验证 UUIDv7、设备证明和 HTTPS 来源；应用层再验证 Agent 存在、状态和 Owner/Operator 权限。
- 成功响应强制 `Cache-Control: no-store`，只包含安全投影、验证状态、变化分类和缓存时间。不返回规范化摘要、原始签名、示例、JWKS 或认证私有配置。
- 失败映射为稳定的结构化错误：无效输入 `400`、无权操作 `403`、不存在 `404`、不可信来源 `422`、依赖不可用 `503`、内部错误 `500`。日志只记录关联 ID、操作和失败枚举。

## 5. 故障与攻击验收

| 验收项 | 结果 |
| --- | --- |
| 官方结构 Fixture | A2A 1.0 多接口 Card 被规范化为安全资料 |
| 不支持版本 | 所有接口不兼容时返回 `UnsupportedProtocol` |
| 未知必需扩展 | 在规范化阶段拒绝，不静默降级 |
| 签名异常 | 有效 Ed25519/JWKS 通过；签名后篡改资料返回 `InvalidSignature` |
| 恶意 JKU | 跨来源 JWKS 在网络请求前拒绝 |
| 恶意 URL / DNS | 私网、云元数据与混合 DNS 答案在建立 HTTP 连接前拒绝 |
| 超大/错误响应 | 非 JSON 内容类型与超限 Content-Length/流式正文均拒绝 |
| 过期 | `now >= expires_at` 时确定派生为 `expired` |
| 能力变更 | 能力面变化与纯资料变化被独立分类 |
| 权限绕过 | Viewer 在任何 Agent Card 网络请求前被拒绝 |
| 并发历史写入 | 真实 PostgreSQL 并发写入后仍只保留 10 份快照 |

## 6. 质量门禁

- `just check` 全量通过：Rust/TypeScript 格式、Clippy `-D warnings`、全目标全特性编译、前端构建、工作区测试、协议一致性、Secret 扫描和 GitHub Actions 固定版本检查。
- `python tools/database.py test` 使用真实 PostgreSQL 验证迁移、快照往返、安全 JSON 投影、90 天/10 份裁剪和并发上限。
- `cargo deny check licenses bans sources` 通过；没有许可证、来源或禁用依赖错误。

## 7. 明确边界

- Agent Card 是外部能力声明，不是 Agent Room 所有权证明，不授予房间权限，也不允许远端自动执行任务。
- 未签名 Card 可以作为明确标记的 `unverified` 公开资料；这不提升其授权级别。
- 本任务完成资料和能力边界，但不发布实例在线状态。状态租约、Matrix State Event、可见性和离线判定属于任务 15。
- 通用 A2A 正式任务调用与宿主工具执行不在本任务范围；任何未来执行入口仍需独立授权与审批。
