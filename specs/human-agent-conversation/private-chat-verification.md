# Agent 加密身份与真实私聊验收

日期：2026-09-06。基于 alpha.19 后的对话生命周期修复；本轮未发布安装包或升级生产服务。

## 行为与边界

保留游戏房间与点击人物打开私聊的交互。浏览器和原生 Agent 现在可以完成官方 Matrix SAS 验证，再交换能够被双方解密的私聊，而不只验证 Matrix 已接受事件。

- 新增会话内的 `agent_room_matrix_security` 工具，包含身份检查、只在身份缺失时建立交叉签名、参与者设备查询和 SAS 发起、轮询、确认、错码、取消。IPC 使用独立的 `MatrixSecurityManage` 能力；工具继续默认要求宿主审批，不作为自动批准的只读工具。
- Bridge 复用该任务已登录的 Matrix 客户端和加密 Store，核心身份状态与安全码确认规则通过独立端口验证。IPC/MCP 均拒绝额外字段，特别是无参数操作中的隐藏重置参数或恢复密钥。
- 每次验证检查双方当前房间成员状态；流程绑定房间、用户和事务，每个客户端最多保留 16 个活动流程，超时回收。浏览器只展示共享已加入房间的参与者请求，接受前重新核对服务端成员状态。
- 浏览器显示 Agent 的完整 Matrix 用户和设备 ID。用户在宿主与浏览器中核对全部三组数字后才能确认，轮询与协议协商不代表信任。数字错误会取消官方 SAS 流程；当前普通对话授权不等于已核对安全码。
- 双方发送前检查自身加密身份和收件人的可信签名设备。验证前明确拒绝，浏览器保留草稿和原提交标识供修复后重试。保留 Web 的 `OnlySignedDevicesIsolationMode`、原生 `OnlyTrustedDevices` 与收到事件的官方验证状态检查。
- 已发布的身份缺失本地私钥时返回 `recovery_required`；不会自动覆盖身份。恢复密钥、访问令牌和私钥不通过 MCP 输入或返回。

使用仓库锁定的 matrix-sdk 0.18.0 与 matrix-js-sdk 42.2.0；原生安全操作采用[官方 SDK 加密与验证接口](https://matrix-org.github.io/matrix-rust-sdk/matrix_sdk/encryption/index.html)。没有新增依赖、持久化迁移或修改既有事件协议。新增 IPC 方法与作用域需配套的新 Bridge/MCP；旧客户端使用旧作用域仍可工作。

## 可重复的真实验收

运行 `just private-chat-integration`，或使用仓库 Python 环境执行 `python tools/private_chat.py`。需要已有开发依赖和 Docker；先停止占用 14173 和 8090 的开发进程。脚本构建本地二进制和 Web HTTPS 生产构建，复用既有隔离基础设施编排，结束时关闭测试会话、清理隔离服务并恢复原开发容器。

测试使用真实 Synapse、OIDC、数据库、对象存储、Bridge、MCP stdio 与 Chromium。Agent 端由测试脚本驱动，全部账号、消息、SAS 自动确认仅属于隔离测试；未操作已安装的生产 Bridge，也不把该脚本当作自主运行的真实 Codex 任务。

本轮脚本退出码 0；浏览器场景 1 项通过（38.5 秒，包含以下流程）：

1. 新 Agent 身份为 `missing`，建立后为 `ready`；重复建立不改变身份。
2. 人类点击大厅人物创建私聊。双方尚未验证时，浏览器拒绝发送、正文不上传、草稿保留；MCP 返回 `bridge.security.peer_verification_required`。
3. 浏览器与原生 SDK 独立生成的三组 SAS 数字一致。未获人类确认的请求被拒绝；故意提交错误数字后，原生和浏览器都进入取消状态。
4. 发起新的验证；比较双方独立数字后完成确认。在 390 × 844 视口核查确认按钮可见、页面无横向溢出。
5. 重试原草稿。Agent 的 MCP 实际读取到已解密的人类正文，浏览器收到 Agent 的加密回复，并显示正确的原消息引用。
6. 关闭并重启 Bridge，以原任务 key 重开会话；Agent、实例和 Matrix 设备保持不变，加密身份仍为 `ready`。浏览器刷新后可读原私聊历史，再次双向收发成功。
7. 四条私聊正文均未出现在公开大厅时间线，Pixi 场景仍正常显示；未捕获意外页面、网络或控制台错误。

机器可读结果在 `artifacts/private-chat/result.json`；截图为同目录的 `sas-mobile.png` 和 `private-roundtrip.png`。这些本机测试产物不纳入提交。运行结束后原来的 8 个开发容器和 14173 游戏房间预览已恢复。

## 回归结果

| 检查 | 实际结果 |
| --- | --- |
| `corepack pnpm@10.28.0 check` | 117 文件、498 项测试通过；格式、ESLint、国际化、类型和协议一致性通过 |
| `corepack pnpm@10.28.0 build` | Web 生产构建通过 |
| `cargo fmt --all --check` | 通过 |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | 通过 |
| `cargo check --locked --workspace --all-targets --all-features` | 通过 |
| `cargo test --locked --workspace --all-features` | 759 项通过，65 项按既有配置忽略 |
| `python -m unittest discover -s tools/tests -p "test_*.py"` | 258 项运行，6 项按既有配置跳过，其余通过 |
| `python tools/plugin.py validate` | 工具声明、审批与插件模板通过 |
| Secret、Actions、许可证、开源材料与 Go/No-Go 记录校验 | 通过；现有发布决策仍为 NO-GO，本轮不改变发布准入结论 |

保留既有打包大小、Windows 链接器信息与 jsdom 滚动 API 提示；没有关闭检查或削弱失败断言。完整检查后仅调整参与者验证弹窗的标题提示，并补跑对应 UI 测试和国际化检查。

## 尚未覆盖

后续已完成桌面 Agent 恢复入口与新设备恢复测试，见[恢复验收](./agent-recovery-verification.md)。下文保留本轮执行时的范围。

尚未完成 Agent 丢失加密 Store 后的可信恢复界面、新设备或身份更换、撤销设备的完整真实联调，以及多个实际 Codex 宿主持续应答。本轮证明已有加密 Store 的重启恢复，不代表丢失密钥后也能恢复历史。安装包发布、生产升级和旧版迁移仍需单独完成。
