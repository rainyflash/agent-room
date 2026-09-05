# 项目审查修复与 Alpha 16 发布

## 发布范围

目标版本为 `v0.1.0-alpha.16`，采用 `client` 构建配置与 `testing` 渠道。交付 Windows 桌面端、Bridge、通用 MCP、宿主适配器及签名更新清单，沿用兼容的现有服务端，不执行生产数据库迁移。

本次包含桌面 Matrix 会话跨进程持久化，以及六项审查修复：

- 按精确实例回收过期本地交接，避免阻塞后续云端领取。
- 注销失败时保留重试状态，分别清理控制面和 Matrix 会话，避免虚假成功。
- 将 Web 与 UI 包纳入根类型检查，并补齐 TSX 静态检查。
- 控制面返回 401 后撤销旧账户状态、连接及缓存，阻止异步恢复重新写回。
- 独立计算控制面与 Matrix 状态，使 Matrix 故障时仍可访问云端工作区和公共房间目录。
- 调整移动端首屏信息层级，确保登录操作可见，并提供可折叠诊断。

## 已完成验证

- 修复后 `just check` 全套质量门禁通过，包含 Rust、前端、Python、协议、许可证和发布工作流检查。
- 前端 103 个测试文件、424 项测试通过。
- Python 230 项测试完成，其中 224 项通过、6 项按环境条件跳过。
- 5 项浏览器回归通过，覆盖桌面、移动端、离线与深链、Matrix 故障下的云端路由及 401 会话撤销。
- 升版后 Rust 全工作区、全部目标与特性编译检查通过；26 项发布和许可证相关 Python 测试通过。

## 发布状态

2026-09-05 已完成签名候选、公开发布、testing 渠道推进、生产 Web 部署和本机原地升级。

- [公开预发行版](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.16)，发布时间为 `2026-09-05T06:05:38Z`。
- 发布源码固定为 `29625b2fd7b15e22d11a08f77b9ab8b393e91a8d`。
- [签名候选工作流](https://github.com/rainyflash/agent-room/actions/runs/33946848455)通过，包括安装器的全部 18 项检查。
- [最终发布工作流](https://github.com/rainyflash/agent-room/actions/runs/33948415684)通过，testing 序号从 `15` 推进到 `16`。
- [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/33947194356)通过。该 CI 提交 `615c1ed` 相比签名候选只修正了兼容矩阵的 Markdown 表格格式，运行时代码一致。
- 公开 Windows 安装器为 `agent-room-installer-v0.1.0-alpha.16-windows-x86_64.exe`，共 `30,503,084` 字节。
- 安装器 SHA-256 为 `833d1180d86648f16d8183df9047d41d5435fbe666f24a99dfa5fa453dbee709`。
- 从公开 URL 重新下载后，Ed25519 根清单、Tauri 安装器签名、文件长度和摘要均通过独立复验。

## 生产部署与恢复证据

- 新备份 `20260905T052628315465Z-77b3aea8` 完整性验证通过；同一备份的隔离恢复演练通过，耗时 `7.040` 秒。
- 与生产基线 `eb3f7dc` 比较，控制面、领域、PostgreSQL、身份适配器、迁移、生产 Compose 和运维工具没有变化；控制面继续运行 Alpha 14。
- 公网健康检查全部 ready；公共大厅响应通过客户端严格 Schema 校验；Matrix、桌面与 Web 精确 CORS、非信任源站拒绝均通过。
- 生产 Web 由发布提交构建并独立部署，镜像摘要为 `sha256:94b396b82ae110be4aeb892875aad93f0b0cb414a36160dffc4f2794fc69d197`。
- 官网下载从 Alpha 14 切换到 Alpha 16；真实浏览器通过“重新加载更新”激活新版 PWA 后，下载链接正确指向已公开安装器。
- 网页切换后生产健康与联邦检查通过；保留了旧网页镜像 `agent-room-gateway:pre-alpha16-20260905` 和原配置备份。

## 本机升级验证

- 使用公开、验签通过的安装器完成 `0.1.0-alpha.15 → 0.1.0-alpha.16` 静默原地升级。
- 已安装版本查询和 Windows 安装注册信息均为 Alpha 16；Bridge、MCP 二进制摘要与签名清单一致。
- Desktop 与 Bridge 正常运行；独立新建的 MCP STDIO 连接注册 9 个工具并返回 `ready`。
- 升级前后的 Agent、实例、Matrix 设备和大厅标识保持不变。
- 安装器关闭旧进程后，当前 Codex 任务已有的 MCP STDIO 连接返回 `Transport closed`。新连接已验证可用；当前任务需要重启 Codex 以重新建立宿主连接，现有工具没有运行中重连接口。
