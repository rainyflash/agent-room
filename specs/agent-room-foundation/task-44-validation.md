# 任务 44 验证记录：开源与自托管发行

## 1. 当前结论

任务 44 的工程发行面已经完成：英文主 README、中文入口、架构与 ADR、自托管指南、贡献与社区治理、安全披露、兼容矩阵、已知限制和 1609 条锁定依赖许可证记录均进入仓库；开发环境检查、配置生成、安装/升级/备份/联邦诊断和干净快照验收均由脚本执行。

任务清单暂不勾选。实际公网 Linux 主机安装仍受任务 40 阻断，任务 42/43 的真实生产演练与受保护签名发行也未完成。在 Windows 上通过 Compose 渲染不能冒充公网 DNS、ACME、联邦和异地备份已经验收。

## 2. 开源发行面

- `README.md` 是英文产品、架构、开发和自托管入口，并在首屏明确公开测试 No-Go；
- `README.zh-CN.md` 提供中文入口，不复制全部英文文档形成第二套事实源；
- `docs/architecture.md` 固定领域、应用、适配器、Bridge、UI 与插件边界；
- 5 个 ADR 记录 Clean Architecture、Matrix 联邦、本地 Bridge、Compose-first 和离线根发布决策；
- `CONTRIBUTING.md`、`CODE_OF_CONDUCT.md` 与 `SECURITY.md` 不含“以后公布”的占位联系渠道；
- `docs/compatibility.md` 区分工程覆盖和公开支持，不把编译通过写成平台支持；
- `docs/known-limitations.md` 公开发行、客户端、联邦、隐私与自托管边界；
- 所有部署示例仅使用 `.example`/`example.com` 保留域名和假邮箱，不包含凭据字段。

## 3. 自动化入口

| 目标 | 命令 | 不变量 |
| --- | --- | --- |
| 开发环境诊断 | `just doctor` | 校验 Git、Rust、Cargo、Node、pnpm、Docker Compose 和 just 版本，不修改工作区 |
| 开发环境准备 | `node tools/bootstrap.mjs` | 同一跨平台实现安装锁定依赖、生成协议并抓取 Cargo 锁定依赖 |
| 自托管配置 | `python tools/self_host.py init ...` | 严格领域校验、`0600`、仅新建、不生成凭据 |
| 主机预检/安装 | `self_host.py doctor/install` | 委派现有 production 执行层，不复制部署逻辑 |
| 升级/健康/联邦 | `self_host.py upgrade/health/federation` | 保持单一生产编排事实源 |
| 备份/核验/恢复 | `self_host.py backup/backup-verify/restore-drill` | 使用任务 41 的原子备份与隔离恢复实现 |
| 开源门禁 | `just oss-check` | 验证必需文件、本地链接、占位符、保留域名、凭据字段和许可证生成物 |
| 干净快照验收 | `just oss-acceptance` | 从 `git archive HEAD` 导出无依赖/无状态副本后执行完整入口 |

默认自托管配置使用内置 PostgreSQL 与对象存储。安装器通过专用迁移容器创建和扩展应用 schema，运营者不需要、也不应手工修改 Agent Room 或 Synapse 数据库。

## 4. 许可证与供应链

`tools/license_inventory.py` 从 `cargo metadata --locked` 与 `pnpm licenses list --json` 生成：

- 888 个 Cargo 第三方包版本；
- 721 个 npm 第三方包版本；
- 共 1609 条去重记录；
- `licenses/third-party.json` 机器可读清单；
- `THIRD_PARTY_NOTICES.md` 人类可读清单；
- 两个锁文件的 SHA-256 绑定；
- 本地路径和 workspace 自有包排除。

CI 的 supply-chain job 同时执行 `cargo deny`、许可证清单一致性、开源门禁与 CycloneDX SBOM，不用手工维护依赖表。

## 5. 本地验证结果

```text
python tools/open_source_acceptance.py
  干净 Git 快照导出通过
  无 node_modules 工作区 bootstrap 通过
  环境复核通过
  OSS 文档/示例门禁通过
  许可证锁文件一致性通过
  self-host 与 production 23 项测试通过
  默认配置生成通过
  生产 Compose 渲染与 docker compose config 通过
  共 8/8 步通过

python -m unittest discover -s tools/tests -p "test_*.py"
  107 项通过

pnpm format:check
  通过

pnpm secrets:check
  736 个文本文件通过

pnpm actions:check
  44 个远程 Action 均固定完整 SHA
```

结构化本地结果写入 ignored 文件 `artifacts/oss/task-44-acceptance.json`，不把机器路径或缓存提交进 Git。CI integration job 在每个干净 runner 上运行同一 bootstrap，并生成默认自托管配置后验证生产 Compose。

## 6. 外部门禁

任务 44 完成状态还要求：

1. 在干净 x86-64 Linux 主机使用公开 DNS 完成 `doctor` 与 `install`；
2. ACME、OIDC、Matrix 客户端入口和联邦委派均从公网验证；
3. 备份 timer 写入异地存储并完成隔离恢复演练；
4. 任务 42 的真实故障演练和任务 43 的受保护签名发行通过；
5. 一名没有开发缓存的外部贡献者按 README 复现开发环境并留下 CI/评审证据。

在这些证据存在前，项目可以开源代码，但不能宣称公开测试发行已经 Go。
