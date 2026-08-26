# 任务 39 验证记录：容量压测与性能预算

## 1. 当前结论

六个真实容量场景的工具与严格汇总门已经实现。候选提交 `d794418fef032f8dba37b7ee947bf2fc045dc40c` 上，PostgreSQL、Matrix、对象存储、双 Homeserver 断网回填和 Chromium WebGL 五个场景全部通过，并且均标记为可用于发布门禁。

任务 39 仍为 **No-Go**。唯一未满足项是 72 小时活跃 Bridge 常驻观测；严格门只拒绝旧修订、未通过且不具备发布资格的 Bridge 报告，没有把五个已通过场景误判为失败。

## 2. 同修订实测容量

| 场景 | 真实依赖 | `d794418` 结果 | 发布资格 |
| --- | --- | --- | --- |
| 目录与分配 | PostgreSQL 18 | 10,000 Agent、1,000 在线实例、250 人大厅；搜索 P95 2.736ms，实例查询 P95 104.356ms | 通过 |
| 大厅消息 | Synapse Client API | 250 成员；持续 10.009 条/秒；突发 66.116 条/秒；650/650 唯一事件送达 | 通过 |
| 内容并发 | SeaweedFS S3 路径 | 4 个并发 25 MiB 对象；上传 P95 1,085.194ms；下载 P95 287.250ms | 通过 |
| 联邦回填 | 两个独立 Synapse/PostgreSQL | 对端离线 1,800 秒；10/10 按序回填；0 重复；回填 1,617.311ms | 通过 |
| Web 场景 | 生产 Vite + Chromium WebGL/CDP | 200 节点；P95 17.5ms；约 236 MiB JS 堆；26 资源；5 纹理 | 通过 |
| Bridge 常驻 | Release Bridge 进程 | 旧修订只做过 0.005 秒、无活跃会话探针 | 不通过 |

联邦实验的可提交摘要见 [`evidence/task-39-federation-outage.json`](./evidence/task-39-federation-outage.json)。六份完整一次性报告写入 ignored 的 `artifacts/capacity/`，避免提交机器路径、进程 ID 和临时 Matrix 标识。

## 3. 30 分钟联邦断网证据

在 `agent-room-federation-test` 隔离 Compose 项目中真实停止 beta Homeserver 1,800 秒。alpha 在此期间保持健康，并按 3 分钟间隔接受 10 个待联邦事件。beta 恢复后：

- 交付率 100%；
- 事件顺序与发送顺序完全一致；
- 重复事件数为 0；
- 停机期间本地写入耗时 123.740ms；
- 对端重启耗时 6,857.820ms；
- 恢复后的回填耗时 1,617.311ms；
- 下一阈值为 60 分钟与 100 个排队事件。

测试结束后脚本删除其专用容器、网络和数据库卷，没有触碰开发或生产项目。

## 4. Web 生产预算

内存和资源体积只在 `AGENT_ROOM_CAPACITY_REPORT=1` 的生产预览中判定；Vite 开发服务器的 HMR 模块与并行测试 worker 不是生产容量证据。当前生产结果：

- 帧耗时中位数 16.7ms，P95 17.5ms；
- 渲染 P95 6.0ms，更新 P95 1.1ms；
- JS 堆 247,363,416 字节，预算 256 MiB；
- 解码资源 2,005,300 字节，预算 12 MiB；
- 26 个资源，预算 80；
- 200 个渲染节点、5 个纹理、0 个外部图片。

主包仍触发构建器的 500 KiB 分块提示，且 JS 堆只剩约 20 MiB 余量。它没有突破当前硬预算，但在提升到下一档 300 节点前必须优先验证按场景拆包和 Matrix Crypto 的延迟加载收益，不能把当前通过外推成无限容量。

## 5. 严格门结果

在 `d794418` 上执行：

```text
python tools/capacity.py gate \
  artifacts/capacity/database-report.json \
  artifacts/capacity/matrix-report.json \
  artifacts/capacity/content-report.json \
  artifacts/capacity/federation-report.json \
  artifacts/capacity/web-report.json \
  artifacts/capacity/bridge-soak-report.json
```

汇总门只报告三项，且全部属于 `bridge_72_hour_soak`：报告不是当前 Git 修订、`passed` 不是 `true`、`releaseGateEligible` 不是 `true`。数据库、Matrix、内容、联邦和 Web 报告均来自同一精确修订并通过。算法模型永远不能满足该门。

## 6. 解除条件

1. 在已完成授权、登录并进入真实大厅的 Agent 会话上构建 Release Bridge；
2. 执行 `python tools/capacity_bridge.py --duration-seconds 259200 --sample-seconds 30 --agent-id <UUIDv7> --catalog-id <UUIDv7>`；
3. 连续运行 259,200 秒，最大 RSS ≤ 512 MiB、增长 ≤ 128 MiB，并保留活跃会话日志与每 30 秒原始样本；
4. 在同一候选修订重跑其余五个真实场景，再执行严格汇总门。

72 小时就是 72 小时。缩短测试、伪造时间戳、只设置环境变量却没有真实登录，或用空闲包装进程替代 Bridge 都不算完成。
