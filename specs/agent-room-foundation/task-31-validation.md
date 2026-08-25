# 任务 31 验证记录：窄权限桌面壳与 Bridge 生命周期

## 1. 结论

任务 31 已完成。桌面壳不是一个能任意碰文件系统和 Shell 的浏览器包装层，而是一个只有四个闭合命令的 Tauri 适配器：读取运行时快照、明确重试 Bridge、切换自动启动，以及打开由原生侧持有的一次性授权地址。Web 业务代码依赖 `DesktopRuntimeGateway`，不直接依赖 Tauri。

Bridge 由原生监督器发现、启动、诊断和关闭。自动重启采用有界指数退避；第四次连续崩溃进入停机态，只有用户明确操作才重置预算。应用退出会先设置原子关闭事实再终止子进程，消除了退出事件与重启计时器之间的竞态。

## 2. 桌面边界

- 主窗口 Capability 只允许事件监听和四个 Agent Room 命令；没有文件系统、Shell、Process 或通配权限。
- 授权界面只收到站点、一次性代码、过期时间和原生提示标识；完整授权 URL 不进入 WebView DOM。
- 深链仅接受已实现的大厅目录与实例路由；外部 URL、查询注入和未实现的 handoff 路由全部拒绝。
- 单实例插件将重复启动收敛到原窗口；托盘恢复、系统通知、休眠恢复和可选自动启动均由原生侧处理。
- Node.js 只参与构建；运行时交付物是 Rust Bridge sidecar，不启动 Node 进程。

## 3. 构建与缓存隔离

普通 Web/PWA 与桌面 WebView 使用两个明确构建模式。`build:desktop` 禁用 PWA 产物，防止 `tauri.localhost` 被旧 Service Worker 长期控制。桌面启动时还执行一次有界迁移：只注销同源 Service Worker 并删除 Cache Storage，不触碰 `localStorage`、认证事实或用户数据，然后刷新到当前内置版本。

Tauri CSP 明确允许自身 IPC 传输 `ipc:` 与 `http://ipc.localhost`，同时继续拒绝任意对象、Frame、外部表单和未列出的网络目标。字体仅允许随包资源和 `data:`，没有开放远端字体域。

## 4. 自动验证

```text
pnpm check
  Prettier、ESLint、TypeScript、协议生成物通过
  60 个 Vitest 文件、221 个测试通过

cargo fmt --all -- --check
cargo clippy -p agent-room-desktop --all-targets --all-features -- -D warnings
cargo test -p agent-room-desktop --all-features
  10/10 通过

pnpm build
  协议包与普通 PWA 生产构建通过

pnpm --filter @agent-room/desktop exec tauri build --debug --no-bundle \
  --config src-tauri/tauri.sidecar.conf.json
  Windows 原生桌面可执行文件构建通过
```

CI 新增 `desktop-platforms` 矩阵，在 `windows-latest` 与 `macos-latest` 上分别构建桌面专用 Web 产物、Bridge sidecar，并检查与测试桌面 crate。当前人工运行证据来自 Windows；macOS 的原生运行与安装包证据仍会在任务 36 的封闭发布矩阵中作为发布硬门禁，不会用 Windows 结果冒充。

## 5. Windows 真实运行时证据

使用实际 `agent-room-desktop.exe` 与 WebView2 验证：

- 启动后旧 PWA 缓存被退役，加载的哈希资源与当前桌面构建一致；控制台不再出现 IPC CSP、字体 CSP 或 `desktop.event.invalid_runtime`。
- Bridge 连续失败后停在 `desktop.bridge.restart_budget_exhausted`，保留最近稳定故障码与退出码，不再循环拉起；点击“重试 Bridge”才重新进入启动态。
- 在同一进程存活时再次启动，只存在一个窗口和一个进程。
- 从真实 DevTools 调用未授权的 `plugin:shell|stdin_write`，运行时明确返回：`shell.stdin_write not allowed. Permissions associated with this command: shell:allow-stdin-write`。
- 本地测试控制平面未启动时，界面保持降级而非假绿色；控制台中的测试证书错误属于未启动本地依赖，不是桌面 IPC 故障。

## 6. 提交

- `b700041`：建立 Tauri 壳、Bridge 监督器、托盘、通知、深链、自动启动与安装清理基础。
- `6583ae5`：绑定类型化 WebView 适配器、收紧运行时权限、隔离桌面构建并退役旧 Service Worker。

## 7. 下一步

下一项是任务 32：把仍散落在组件、原生事件与诊断界面中的可见字符串收敛到类型化中英文消息目录，并补齐账户语言、设备临时覆盖、`Intl` 格式化和机器翻译显式标记。
