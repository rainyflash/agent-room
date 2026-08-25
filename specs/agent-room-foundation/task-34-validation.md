# 任务 34 验证记录：可靠性与离线恢复

## 1. 结论

任务 34 已完成。消息发送的唯一真相仍是 Bridge 持久化提交记录与 Matrix 事务观察，浏览器没有增加一套会盲目重放 POST 的离线队列。断网、旧 Service Worker、本地状态盘不可写、响应未知、进程重启、系统唤醒、Matrix 同步缺口和对象存储清理失败都会进入明确状态，不会显示假绿色“已连接”或把失败冒充成功。

## 2. 写入与恢复边界

- `MessageSubmissionId` 与稳定 Matrix transaction ID 在正文上传前持久化认领；并发认领只有一个创建者。
- Matrix 已接受但响应丢失时进入 `submit_unknown`，后续只通过同步观察同一 transaction ID 对账，绝不猜测重发。
- 事件接受而正文绑定暂时失败时进入 `accepted_binding_pending`；重试只补绑定，不产生第二个可见事件。
- Matrix `limited` timeline 被持久化为同步缺口，游标与投影在一个 SQLite 事务中提交。
- 孤儿、过期和撤回内容由清理用例分批回收；单对象失败保留为可重试事实，不阻断整个批次。
- 本地状态盘暂时不可用时，发送在任何正文上传和 Matrix 发布之前失败；恢复后同一意图只产生一个事件。

这些约束复用了任务 11、13、17 和 18 已建立的发送、同步、清理与 Bridge 会话边界，没有在 React 组件里复制业务状态机。

## 3. PWA 版本安全

Workbox 报告有等待激活的新版本后，`RuntimeCompatibilityProvider` 会把当前页面永久锁为只读，直到用户应用更新并重载。提示不能被关闭来绕过兼容门。离线状态同样禁止新写入；生产 Service Worker 只预缓存 GET 资源，没有生成 Background Sync/POST 重放队列。

采用该策略是因为 Chrome 官方 [Service Worker 生命周期](https://developer.chrome.com/docs/workbox/service-worker-lifecycle/)明确说明旧页面可能继续由旧 worker 控制，而 [Workbox Background Sync](https://developer.chrome.com/docs/workbox/modules/workbox-background-sync/)会把失败请求放入 IndexedDB 后重放。对版本化协议写入而言，盲目重放比明确只读更危险。

## 4. 六类依赖故障能力矩阵

能力决策由纯领域策略合并；多个故障同时出现时取最严格状态，并保留决定性原因。

| 故障 | 仍可用 | 只读/替代 | 阻断 |
| --- | --- | --- | --- |
| 控制平面 | 已建立 Matrix 会话内发送 | 已缓存大厅 | 新认证、进房、按需正文票据 |
| Matrix | 已缓存预览与本地设置 | 已缓存大厅 | 进房、发送、Agent 在线工具 |
| 对象存储 | 大厅与既有预览 | — | 正文读取、需要内容上传的发送 |
| OIDC | 现有未过期会话 | — | 新认证与会话恢复 |
| Bridge | Web 观察与人工浏览 | — | Agent 工具和 Agent 自动发送 |
| Pixi/GPU | 功能完整 DOM 列表 | 可视大厅被列表替代 | — |

网络在线只说明浏览器网络接口存在，不代表控制平面、Matrix 或 Bridge 可用。Web 会话状态机、健康条和桌面 Bridge 生命周期分别展示其真实边界。

## 5. 故障注入门禁

`python tools/reliability.py` 统一执行五组可重复场景，并把报告与原始日志写入被 Git 忽略的 `artifacts/reliability/`：

```text
浏览器断网与旧协议写入门       19/19 通过
Bridge 断网、未知提交与同步缺口 15/15 通过
本地状态跨进程恢复             10/10 通过
休眠唤醒与崩溃预算             11/11 通过
孤儿内容失败隔离与重试          3/3 通过
```

休眠恢复不沿用休眠前的 ready 快照：桌面收到 `RunEvent::Resumed` 后必须重新探测 Bridge；探测缺失时启动受管进程，阻断时进入 halted，现有受管进程尚未恢复时显示 pending。连续崩溃采用 1/4/16 秒退避，第四次停止自动重启并要求人工诊断。

## 6. 全量门禁

```text
pnpm check
  Prettier、ESLint、i18n、TypeScript、协议生成物通过
  66 个 Vitest 文件、247 个测试通过

pnpm build
  协议包与 Web/PWA 生产构建通过

cargo clippy -p agent-room-bridge-core -p agent-room-desktop --all-targets --all-features -- -D warnings
  通过
```

## 7. 提交

- `fd3f35c`：建立版本安全写入门、依赖能力矩阵、持久恢复故障夹具和统一可靠性门禁。

## 8. 下一步

下一项是任务 35：以安全文档第 19 节为硬门禁，完成浏览器、协议、IPC、附件、远端提示注入和敏感数据泄漏的封闭测试加固。
