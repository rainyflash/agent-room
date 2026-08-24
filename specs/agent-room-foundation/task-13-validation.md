# 任务 13 Bridge 守护进程基础验证记录

> 验证日期：2026-08-24
>
> 结论：通过
>
> 对应任务：[实施计划 13](./tasks.md#m1内部纵向切片)
>
> 核心实现提交：`608fdfd`、`ac933cc`、`84f80b3`、`d66b8c5`、`106b21f`、`e77ad9b`、`fbec1a5`

## 1. 架构边界

- `bridge-core` 只包含 IPC 协商、状态转换、退避和会话续期等纯策略，不依赖 Tokio 监听器、Windows ACL、Unix 权限、Matrix SDK 或操作系统密钥环。
- `apps/bridge` 是唯一进程组合根，负责装配运行目录、锁、私有 IPC、密钥环、Matrix Store 和控制平面设备会话。平台细节没有泄漏到领域或应用层。
- `matrix-adapter` 只封装标准 Matrix Client-Server SDK；Application Service 身份签发拆到 `matrix-provisioning-adapter`。这避免控制平面无意义地携带客户端 Store、SQLite 与同步依赖。
- 进程状态通过单一状态快照暴露为 `ready`、`reconnecting` 或 `offline`。IPC Handler 只读取快照并映射协议，不自行推断连接状态。

## 2. 私有 IPC 与调用方认证

- Windows 命名管道使用显式 SDDL，只允许当前登录会话 SID 与 `SYSTEM`，拒绝依赖进程默认 DACL；ACL 不包含 Everyone、Anonymous、Authenticated Users 或 Builtin Users。
- macOS/Linux 在私有 `0700` 运行目录创建 `0600` Unix Socket。Bridge 不打开 TCP、UDP 或公网监听器。
- 客户端先声明调用方类型、支持版本和封闭枚举作用域；服务端选择兼容版本后发送一次性随机 Challenge。客户端必须使用安装级共享密钥提交 HMAC 证明，安装 ID、版本、作用域和 Challenge ID 均纳入证明。
- 握手具有 5 秒超时，未知版本、越权作用域、错误 Challenge、错误安装 ID、超大帧和协议乱序全部返回稳定结构化错误并关闭连接。
- 安装级共享密钥和 Matrix Store 口令只进入操作系统密钥环；配置、错误调试输出和日志均不含明文密钥、消息正文或宿主工作区路径。

## 3. 单实例、Store 与生命周期

- Bridge 启动前分别取得守护进程锁和 Matrix Store 独占锁。第二个进程得到确定的 `AlreadyHeld`，不会争用命名管道或同时写 Store。
- Matrix SDK 使用绝对私有路径和加密 SQLite Store；口令不少于 32 字节。启动时真实初始化 Store，错误口令会失败，不允许静默退回内存 Store。
- Ctrl+C 先停止接受新 IPC，再等待连接任务收束并释放锁和 Socket。Unix 遗留 Socket 只在确认没有活跃锁持有者后清理。
- Bridge 维护短期控制平面设备会话，在访问令牌到期前主动刷新。瞬时不可用使用有上限的等抖动指数退避；成功后重置退避。
- 休眠跨过续期时间会立即刷新。网络切换导致的暂时失败进入 `reconnecting`，恢复后回到 `ready`；授权失效、刷新结果不确定或本地安全存储损坏则进入 `offline`，不会盲目重放旧刷新令牌。

## 4. 故障与攻击验收

| 验收项 | 结果 |
| --- | --- |
| 第二 Bridge 进程 | 启动真实子进程持有守护进程锁，父进程得到 `AlreadyHeld` |
| Matrix Store 锁冲突 | 启动真实子进程持有 Store 锁，父进程无法取得写入权 |
| 错误用户 IPC | Windows ACL 构造与解析测试确认只含当前登录会话 SID 和 `SYSTEM`；内核在连接前执行 DACL |
| IPC 伪造与降级 | 错误 HMAC、Challenge ID、安装 ID、版本和调用方作用域均被拒绝 |
| Store 口令错误 | 已创建的真实 SQLite Store 使用错误口令重新打开时失败 |
| 休眠恢复 | 模拟当前时间越过续期截止点，恢复计划立即触发刷新 |
| 网络切换 | 首次控制平面不可用进入重连，下一次成功恢复凭据和 `ready` 状态 |
| 不确定刷新结果 | 不重放旧刷新令牌，转为需要重新授权的离线状态 |

错误用户场景采用 ACL 语义和受保护对象的自动化测试，没有在 CI 中创建第二个真实 Windows 账户。这里明确记录验证层级，避免把 ACL 单元验收冒充成交互式跨账户端到端测试。

## 5. 质量门禁

- `just check` 全量通过：Rust 与 TypeScript 格式、Clippy `-D warnings`、ESLint、全目标全特性编译、类型检查、前端构建、工作区测试、协议一致性、Secret 扫描和 GitHub Actions 固定版本检查。
- Bridge 测试为 25 个通过、1 个真实环境场景忽略；`bridge-core` 的 IPC、认证、状态和续期策略测试全部通过；`matrix-adapter` 的 12 个单元测试及普通集成测试通过。
- `cargo deny check licenses bans sources` 通过。重复依赖仅报告警告，没有许可证、来源或禁用依赖错误。
- SQLx 与 Matrix SDK 对 SQLite 原生依赖的版本范围在锁文件中统一到兼容版本，没有引入两份冲突的 `libsqlite3-sys` 链接目标。

## 6. 明确边界

- 本任务初始化并保护 Matrix SDK 持久 Store，但尚未让某个 Agent Matrix Device 常驻同步。Agent Card、实例能力绑定和状态租约属于任务 14–15；完成这些业务身份后才把 Matrix 同步会话接入现有守护监督器。
- 当前自动重连覆盖 Bridge 的控制平面设备会话。退避、休眠恢复和状态模型可复用于后续 Matrix 同步，但本记录不虚构尚未存在的 Matrix Sync Loop。
- 私信和私人房间的交叉签名、设备验证与 E2EE 恢复属于任务 27。持久 Store 加密不能替代端到端加密验收。
