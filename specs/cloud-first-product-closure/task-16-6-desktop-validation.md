# 任务 16.6.3：真实 Windows 桌面云端闭环证据

## 结论

真实 Windows Tauri/WebView2 进程已经完成云端优先闭环验收。验收故意把桌面端管理的 Bridge 指向不可达端点，直到 Runtime 明确进入 `halted`；在此条件下，人类桌面会话仍可独立读取 Control Plane 与 Matrix、浏览账号工作区，并创建和进入真实公共大厅。

这证明桌面端现在是“云端客户端 + 可选本机 Runtime”，而不是 Bridge 的遥控页面。Bridge 离线只降级 MCP、宿主配置和本机 Agent 能力，不再截断账号级云端产品。

## 可重复命令

在仓库根目录使用项目虚拟环境运行：

```powershell
.\.venv\Scripts\python.exe tools\desktop_cloud_acceptance.py
```

脚本会自动完成以下操作：

1. 构建当前工作树的 Control Plane、Bridge、通用 MCP 和 Tauri 调试壳；
2. 创建全新 PostgreSQL、Matrix、OIDC、对象存储和 Gateway 隔离环境；
3. 迁移数据库并写入幂等测试账户和公共大厅目录；
4. 启动真实 Tauri 进程和 WebView2 调试端口；
5. 建立独立的浏览器 OIDC 会话、桌面 PKCE 会话和 Matrix 设备会话；
6. 故意阻断原生 Bridge 后端，并等待 Runtime 进入 `halted`；
7. 通过真实指针交互进入 Control Plane 即时配置的 Matrix 公共大厅；
8. 关闭全部进程、删除隔离数据卷和随机 Windows 凭据命名空间，并恢复原开发容器；
9. 反向扫描落盘日志，发现密码、JWT、设备码或已知凭据时直接失败。

## 机器可验证结果

成功结果写入被 Git 忽略的 `artifacts/desktop-acceptance/desktop-cloud-closure.json`。本次结果为：

| 断言 | 结果 |
| --- | --- |
| 进程类型 | `tauri_webview2` |
| Tauri Runtime | 已检测 |
| 页面来源 | `http://tauri.localhost` |
| Control Plane | `online` |
| Matrix | `online` |
| Bridge | `halted` |
| 账号工作区 | 可见 |
| 公共大厅 | `Vertical Codex Lobby`，已进入 |

验收报告只包含枚举状态和公共夹具名称，不包含会话、访问令牌、密码、设备密钥或本机临时目录。

## 验收中发现并修复的问题

第一次从空环境执行时，Playwright 在大厅目录异步请求完成前读取了占位符 `—`。失败截图最终已经显示正确大厅名称，说明产品已完成加载、测试断言却存在竞态。测试现改为等待非占位名称，第二次从空数据卷完整执行通过；没有通过增加固定延时掩盖竞态。

桌面 Runtime 的 WebView2 在受管进程树关闭时会输出 Windows 类注销错误 `1411`，但进程退出、凭据清理和证据验证均成功。它是调试 WebView 的关停噪声，不计为产品运行失败。
