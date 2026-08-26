# 任务 36 验证记录：封闭测试发布与验收

## 1. 当前结论

提交 `f5867fe2ac04511f1d77f7d8bee11f3938e7a274` 的本地 M2 矩阵已经 **6/6 通过**，没有开放的产品阻断项，也没有敏感输出违规。质量、浏览器旅程、真实多用户交接、三设备 E2EE、恢复和隐私安全场景均基于同一干净修订执行。

任务 36 仍为 **No-Go**，因此清单不勾选：当前修订没有同时产出 Windows x86-64 与 macOS arm64 桌面测试包，GitHub 上也没有完成 M2 签字门。不能用本机通过替代跨平台制品验收。

## 2. 当前修订矩阵

| 场景 | 结果 | 时长 | 主要覆盖 |
| --- | --- | ---: | --- |
| 全工作区质量 | 通过 | 112.400 秒 | 格式、Lint、类型、构建、单元、协议、许可证、Secret 与 Action 固定 |
| 浏览器旅程 | 通过 | 19.858 秒 | 中英文、无障碍、私人房间、治理、消息与 200 节点场景 |
| 多用户与 Agent | 通过 | 187.135 秒 | 真实服务、Bridge、Codex 插件、状态、私信与一次性交接 |
| 多设备 E2EE | 通过 | 132.087 秒 | 三设备交叉签名、SAS、加密房间与恢复 |
| 故障恢复 | 通过 | 8.290 秒 | 断网、休眠、重启、未知提交与磁盘暂时不可用 |
| 隐私安全 | 通过 | 9.344 秒 | 输入边界、提示注入、内容、限流与敏感输出 |

可提交的最小结构化摘要见 [`evidence/task-36-m2-local.json`](./evidence/task-36-m2-local.json)。完整日志和 Playwright trace 写入 ignored 的 `artifacts/closed-test/`，避免把机器路径与一次性测试状态提交到 Git。

## 3. 跨平台制品门

`.github/workflows/closed-test.yml` 定义三个不可跳过的部分：Ubuntu 完整矩阵、Windows x86-64/macOS arm64 桌面制品，以及依赖两者成功的 M2 签字门。每个平台必须包含 Web、Codex 插件、Desktop 和单机部署包，并绑定同一 Git 修订与 SHA-256。

当前事实：

- 本机仅有旧修订的 Windows 清单，不能用于当前候选；
- 当前工作区没有 macOS arm64 制品；
- GitHub Actions 运行 [32910476274](https://github.com/rainyflash/agent-room/actions/runs/32910476274) 的四个 job 均未开始执行，平台返回账户付款失败或 spending limit 不足；
- 本地 `main` 尚未推送当前候选，因此没有当前修订的远端 M2 运行。

这属于发行基础设施阻断，不是测试失败；但对发布结论同样是硬阻断。

## 4. 已发布边界

- [`release/closed-test/DATA-POLICY.md`](../../release/closed-test/DATA-POLICY.md) 说明保留、导出、删除和联邦残留；
- [`release/closed-test/KNOWN-LIMITATIONS.md`](../../release/closed-test/KNOWN-LIMITATIONS.md) 说明客户端、容量、联邦与签名边界；
- [`release/closed-test/SECURITY-BOUNDARIES.md`](../../release/closed-test/SECURITY-BOUNDARIES.md) 固定本地 Bridge、E2EE、自动发言和远端内容边界；
- [`release/closed-test/blockers.json`](../../release/closed-test/blockers.json) 中五个已发现产品阻断项均有根因与关闭记录。

## 5. 解除条件

1. 解决 GitHub Actions 计费/额度问题并推送精确候选修订；
2. 同一 workflow run 的完整矩阵、Windows x86-64 和 macOS arm64 job 全部成功；
3. 下载聚合制品，运行 `closed_test.py verify` 并核对两平台 Desktop 摘要；
4. 由维护者在可审计记录中完成 M2 签字。

在上述四项完成前，外部联邦和公开测试均不得启用。
