# 任务 36 验证记录：封闭测试发布与验收

## 1. 结论

任务 36 已完成。候选提交 `d794418fef032f8dba37b7ee947bf2fc045dc40c` 的 [M2 运行 32986934905](https://github.com/rainyflash/agent-room/actions/runs/32986934905) 在同一次工作流中完成 Ubuntu 验收矩阵、Windows x86-64 制品和 M2 签字门，三个 Job 全部成功。

结构化摘要见 [`evidence/task-36-m2-github.json`](./evidence/task-36-m2-github.json)。摘要固定工作流运行号、精确 Git SHA、原始报告 SHA-256 和四类制品摘要；下载后的完整产物还在独立的 `d794418` 工作树执行 `python tools/closed_test.py verify --required-platform windows-x64` 并通过。没有用旧候选、本机包或截图替代远端证据。

## 2. 同修订验收矩阵

| 场景 | 结果 | 时长 | 主要覆盖 |
| --- | --- | ---: | --- |
| 全工作区质量 | 通过 | 605.181 秒 | 格式、Lint、类型、构建、单元、协议、许可证、Secret 与 Action 策略 |
| 浏览器旅程 | 通过 | 81.798 秒 | 中英文、无障碍、私人房间、治理、消息与 200 节点场景 |
| 多用户与 Agent | 通过 | 406.975 秒 | 真实服务、Bridge、Codex 插件、状态、私信与一次性交接 |
| 多设备 E2EE | 通过 | 257.188 秒 | 三设备交叉签名、SAS、加密房间与恢复 |
| 故障恢复 | 通过 | 135.604 秒 | 断网、休眠、重启、未知提交与磁盘暂时不可用 |
| 隐私安全 | 通过 | 116.313 秒 | 输入边界、提示注入、内容、限流与敏感输出 |

结果为 6/6，通过时没有开放产品阻断项，也没有敏感输出违规。完整日志和 Playwright Trace 保存在 GitHub Actions 的受限验收 Artifact 中；仓库只提交脱敏摘要，避免写入机器路径与一次性测试状态。

## 3. Windows 首发制品

| 制品 | 平台 | 大小 | SHA-256 |
| --- | --- | ---: | --- |
| Web | all | 10,389,367 字节 | `47d10131f29f4ea1a217a12c71fbc4fc7fabf1a04f4fa1fff4954857c62f62ec` |
| 单机部署包 | linux | 1,885,319 字节 | `02794af93f8ac6ea06d6e5cb835c85accb16c2ee94f0f79be92a943cf02bf521` |
| Codex 插件 | windows-x64 | 2,020,959 字节 | `42ed1d495a441c5e48170b484312ab3d3ade6714979d39bbfab135f777a5c16c` |
| Desktop | windows-x64 | 67,319,750 字节 | `f87a8ebf9d27542d5e24452c656daf8cb8a397cd2c9fe96a00bc449f00615059` |

M2 签字门从同一运行下载验收报告和 Windows 制品，重新验证修订、平台、清单、文件存在性与摘要后才成功。`manifest.json` 的 SHA-256 为 `09fe030c060ef9137167384d18b35f9d6c67323255756f86db8598db5c598540`。

## 4. 平台与成本边界

Windows x86-64 是首发原生目标。自动 CI、M2 和发布候选工作流只使用 Linux/Windows；Actions 静态门禁会拒绝 GitHub 托管的 `macos-*` Runner。macOS ARM64 只保留手动 `.github/workflows/macos-self-hosted.yml`，并且必须命中 `[self-hosted, macOS, ARM64]`，不消耗托管 macOS 3-core。

## 5. 主线回归

修复本地网关绑定后，提交 `e9fff76aea8f75bd71f9c18c1617a7265c168722` 的[主 CI 运行 32992189339](https://github.com/rainyflash/agent-room/actions/runs/32992189339)六个 Job 全部成功，其中真实 OIDC、Matrix SSO 与浏览器会话恢复步骤通过；同一提交的 [CodeQL 运行 32992144789](https://github.com/rainyflash/agent-room/actions/runs/32992144789)对 JavaScript/TypeScript、Python 和 Actions 的分析全部成功。结构化摘要见 [`evidence/task-36-main-regression-github.json`](./evidence/task-36-main-regression-github.json)。

## 6. 已发布边界

- [`release/closed-test/DATA-POLICY.md`](../../release/closed-test/DATA-POLICY.md) 说明保留、导出、删除和联邦残留；
- [`release/closed-test/KNOWN-LIMITATIONS.md`](../../release/closed-test/KNOWN-LIMITATIONS.md) 说明客户端、容量、联邦与签名边界；
- [`release/closed-test/SECURITY-BOUNDARIES.md`](../../release/closed-test/SECURITY-BOUNDARIES.md) 固定本地 Bridge、E2EE、自动发言和远端内容边界；
- [`release/closed-test/blockers.json`](../../release/closed-test/blockers.json) 中五个封闭测试产品阻断项均有根因与关闭记录。

任务 36 完成只解除 Windows 同源 M2 阻断，不等于公开测试 Go。独立安全评审、72 小时活跃 Bridge、公网部署、生产故障演练、离线根密钥发行和外部干净复现继续由 Go/No-Go 独立阻断。
