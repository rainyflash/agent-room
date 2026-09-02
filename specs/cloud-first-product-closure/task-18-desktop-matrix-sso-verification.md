# 任务 18：桌面 Matrix SSO 与会话恢复验收

## 结论

桌面 Matrix SSO 的主窗口导航泄漏已经修复，并于 2026-09-02 随 `v0.1.0-alpha.14` 完成真实 Windows 升级验收。身份提供方只在系统浏览器中运行，Agent Room 主 WebView 始终保留产品路由；登录完成后，Desktop、Bridge、Matrix SDK 与通用 MCP 均恢复到可用状态。

## 不可变发布边界

- 公开预发行版：<https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.14>；
- 发布提交：`eb3f7dc01ea5cef1fc0d98d46e3522f37cb82c61`；
- 候选工作流：<https://github.com/rainyflash/agent-room/actions/runs/33614402525>；
- 最终发布工作流：<https://github.com/rainyflash/agent-room/actions/runs/33617711465>；
- Windows 安装器：`agent-room-installer-v0.1.0-alpha.14-windows-x86_64.exe`；
- 安装器字节数：`30,499,314`；
- 安装器 SHA-256：`7e7721474f20d63871e8a9374d3f6eb87cfdef3d437a8a99d09561c56542d808`。

## 真实设备验收

- 用户通过系统浏览器完成 Agent Room 与 Matrix 登录，Desktop 主窗口没有导航到 Keycloak、Synapse 或回环回调页；
- 从公开 Release 重新下载安装器，摘要与 Release 资产摘要完全一致；
- 在保留本机状态的前提下完成 `0.1.0-alpha.12 → 0.1.0-alpha.14` 原地升级；
- 正式 Desktop 自动拉起正式 Bridge，既有 Agent、实例、Matrix 用户、Matrix 设备和公共大厅标识均保持不变；
- 已安装通用 MCP 返回 9 个工具，`agent_room_get_self` 的 `connectionState` 为 `ready`；
- Synapse 记录同一 Matrix 设备完成大厅加入、增量同步和状态发布，相关请求返回 `200`；升级后未出现新的 `/keys/upload` `400`，也未产生新的恢复备份目录。

## 密钥冲突恢复

Matrix Rust SDK 会把后台一次性密钥上传冲突持久化为 `OneTimeKeyAlreadyUploaded`。旧实现只观察同步调用的直接返回值，因此可能把已经记录的密码学冲突误判为同步成功。Alpha 14 在每次 `sync_once()` 后读取 SDK 持久化错误，并将该冲突提升为可恢复故障：

1. Bridge 隔离旧 Matrix crypto store；
2. 通过受签名保护的专用会话端点轮换同一 Agent 实例的 Matrix 设备会话；
3. 以同一 Matrix 用户和设备标识创建干净 store；
4. 重新初始化加密、同步并进入 `ready`。

真实故障样本已完成一次恢复，备份位于本机 `matrix-store/recovered-store-backups/01a0615e-fb95-7532-abe2-a846ecc5ffbc`。随后候选安装、公开安装和 Desktop 重启均未再次进入恢复循环。

## 发布与生产证据

- 生产环境运行同一发布提交，健康检查、联邦检查和精确 CORS 验证通过；
- Release 内的数据库扩展、兼容服务与客户端发布晋级记录已由 SHA-256 串联；
- Windows 候选机完成干净安装、运行时启动、MCP 协议、升级与卸载验收；
- 本次候选与发布均未使用托管 macOS Runner。
