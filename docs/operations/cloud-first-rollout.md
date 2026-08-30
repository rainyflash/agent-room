# 云端优先版本发布 Runbook

> 当前状态：`v0.1.0-alpha.8` 已按本文顺序完成生产迁移、发布与公开资产复验。本文继续作为后续云端优先版本的规范发布顺序；本次不可变证据见文末执行记录。

## 目标与不可破坏约束

本次发布把 Web 与桌面端统一为云端客户端，并把 Bridge/MCP 降为本机增强层。发布必须保证旧客户端在兼容窗口内继续工作，新客户端只有在云端 API、Matrix 和精确 CORS 已验证后才可公开。

禁止事项：

- 禁止先发布 Windows 客户端再补服务端；
- 禁止在携带凭据的 CORS 上使用 `*`；
- 禁止事故回滚时执行破坏性数据库 down-migration；
- 禁止用生产访问令牌、OIDC code、Matrix 凭据或 Bridge Secret 作为验证证据。

## 发布前门禁

1. 固定待发布 Git SHA，工作树必须干净。
2. 对当前生产执行备份、`backup-verify` 和隔离 `restore-drill`，记录脱敏证据。
3. 确认当前公开客户端版本、服务端镜像摘要和数据库迁移版本。
4. 确认这两个有序、可向后兼容的迁移包含在候选中：
   - `infra/migrations/202608300001_desktop_human_sessions.sql`
   - `infra/migrations/202608300002_targeted_handoffs.sql`
5. 确认生产 Compose 将 `AGENT_ROOM_DESKTOP_ORIGIN` 精确设置为 `http://tauri.localhost`。
6. 运行仓库完整格式、Lint、类型、Rust、协议、Python、Web 构建和真实 Windows/Tauri Bridge-offline 验收。

任何一项失败都必须 No-Go，不能靠“先发出去看看”绕过。

## 唯一允许的部署顺序

1. 完成并验证生产备份。
2. 部署两个加法迁移、兼容的控制平面和精确桌面源站配置。
3. 验证控制平面、OIDC、Matrix、公共大厅、交接队列以及浏览器/Tauri CORS 预检。
4. 部署 Web 静态资源。
5. 使用无 Bridge 浏览器、多账号浏览器和现有 Windows 客户端观察兼容窗口。
6. 云端稳定后，才构建、签名并发布下一版 Windows Alpha prerelease。
7. 安装公开资产，重新运行同版本桌面验收，再更新发布证据。

## CORS 核验

从任意可访问生产 API 的机器执行预检，路径应替换为真实存在且允许 `OPTIONS` 的 API 路径：

```bash
curl -i -X OPTIONS 'https://api.room.the-zeroth.com/v1/agents' \
  -H 'Origin: http://tauri.localhost' \
  -H 'Access-Control-Request-Method: GET' \
  -H 'Access-Control-Request-Headers: authorization,content-type'
```

响应必须同时满足：

- `Access-Control-Allow-Origin: http://tauri.localhost`
- `Access-Control-Allow-Credentials: true`
- 允许请求所需的方法和 Header
- 不返回 `Access-Control-Allow-Origin: *`

还要用 `https://app.room.the-zeroth.com` 重复预检，证明 Web 源站没有被桌面配置覆盖。

## 观察与 Go/No-Go

Go 必须同时满足：

- 控制平面与 Matrix 健康；
- Web 在 Bridge 不存在时可登录、查看工作区、入厅、发消息和查看交接；
- 同账号两台设备看到一致 Agent/实例/设备投影；
- 第二账号可以加入公共大厅并发送跨账号消息；
- Windows Tauri 在 Bridge 被故意停用时仍可使用云端工作区；
- Bridge 恢复后 MCP 能列出并显式消费定向交接；
- 日志与验收产物通过 Secret 扫描。

任一核心路径失败即 No-Go。局部本机能力失败只能在明确标记为“Bridge 离线”的情况下降级，不能污染云端健康状态。

## 回滚策略

### Web

把静态站点回滚到上一份已验证资产。Web 不持有数据库迁移，因此可独立回滚。

### 控制平面

回滚到仍能容忍新增表/列的上一兼容镜像，保留已执行的加法 schema。不要删除 `desktop_human_sessions`、定向交接表或新增列；破坏性逆迁移会扩大事故面。

### Windows 客户端

未发布前直接停止发布。若 prerelease 已公开，不覆盖或删除同版本资产，也不降低签名序列；发布更高版本的前向修复，并在 Release 中明确受影响版本。服务器必须继续维持已公开客户端所需的兼容 API，直到修复覆盖完成。

### 身份与 Matrix

不要回滚已经签发的人类会话、Matrix 设备或房间事件。撤销异常会话并回滚应用逻辑；Matrix 事件按协议追加补偿，不能修改历史。

## 发布证据

最终记录必须包含 Git SHA、镜像/资产摘要、备份与恢复演练 ID、迁移版本、CORS 响应头、健康检查、浏览器/桌面验收结果和回滚点。全部证据必须脱敏，发布状态写入 `specs/cloud-first-product-closure/task-17-release-candidate.md`。

## 2026-08-30 执行记录

- `v0.1.0-alpha.8` 已从提交 `f8754a5ae1bee195bf07d28f44476663b60c02cd` 构建并作为 Windows testing prerelease 公开；
- 生产备份、隔离恢复演练、两项加法迁移、Control Plane、Matrix、对象存储和精确 CORS 均通过；
- 官网 PWA 从旧 Service Worker 显式提示更新，激活后下载地址切换到 `alpha.8`；
- 公开安装器在候选 Windows 环境和本机原地升级场景中均通过，MCP 真实暴露 9 个工具；
- 完整摘要、工作流、镜像、回滚点和验收结果记录在 [`task-17-release-candidate.md`](../../specs/cloud-first-product-closure/task-17-release-candidate.md)。
