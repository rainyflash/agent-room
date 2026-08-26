# 任务 43 验证记录：签名发布与安全更新

## 1. 当前结论

任务 43 的工程实现已完成：所有发布物进入统一的逐件 SBOM、摘要、Sigstore 证明和离线根清单；桌面端强制双重验证 Tauri 签名与根清单摘要；稳定/测试渠道、单调序号、显式回滚、安装中断恢复及插件/Bridge 不兼容状态均有实现与回归测试。

任务清单暂不勾选。候选工作流尚未在真实 GitHub 仓库完成首轮受保护环境运行，生产离线根密钥也不能由开发会话代替维护者举行密钥仪式。任务 36 和任务 40 仍为前置 No-Go；没有这些外部证据时声称“安全发行已通过”是造假。

## 2. 架构边界

- `release-manifest` 是纯领域 crate，只认识渠道、版本、序号、时效、产物证据和信任状态，不依赖 Tauri、GitHub 或文件系统；
- `release-tool` 是断网 CLI，负责密钥生成、根清单签名和签名验证；私钥文件使用新建语义，Unix 权限 `0600`，敏感缓冲使用 `zeroize`；
- Desktop Adapter 只在领域清单通过后调用 Tauri Updater，并再次核对目标 URL、版本、平台、SHA-256、长度和 minisign；
- pending 安装记录先落盘，目标版本实际启动才推进可信序号；
- `tools/release.py` 与 `release_assets.py` 负责批量路径约束、逐件证据、跨平台聚合和发布候选复验；
- `release_promotion.py` 把数据库扩展、兼容服务端、客户端、观察窗口和旧路径收缩建模为不可跳步历史；
- GitHub 候选和最终发布拆成两个受保护工作流，根私钥从未进入 CI。

## 3. 供应链覆盖

每个候选必须至少包含：

- `control-plane`、`identity`、`web` 三个 amd64/arm64 OCI Index；
- Windows、macOS 的 Bridge；
- Windows、macOS 的 Tauri 更新归档；
- 对应平台 Codex 插件；
- 跨平台 Tauri 更新 JSON。

缺失 CycloneDX、Sigstore bundle、HTTPS 证明 URL、不可变 OCI digest、有效平台名或唯一产物身份均在根签名前失败。最终工作流以候选工作流在 `main` 上的精确 OIDC Identity 验证 bundle，不接受任意 GitHub Workflow 证书。

## 4. 攻击与恢复验证

Rust 测试覆盖：

- 修改 payload 后 Ed25519 验证失败；
- 重放相同/更低序号失败；
- 未精确声明 `rollbackFrom` 的降级失败；
- 过期、未来时间、超长有效期失败；
- 无效摘要、长度、URL、Tauri 元数据失败；
- 只有安装成功并在新版本启动后才能提交信任状态。

Python 测试覆盖：

- 产物路径逃逸候选根目录失败；
- 缺失 SBOM/签名、重复身份、修改文件失败；
- OCI 只接受小写 SHA-256 不可变引用；
- Tauri 归档歧义和平台重复失败；
- 工作流标签必须匹配 workspace 版本；
- 发布阶段跳跃和删改历史失败。

插件与 Bridge 的 IPC 版本不兼容映射为 `bridge.ipc.version_incompatible`，Codex MCP 给出整套升级指引，Desktop Supervisor 不启动残缺工具。

## 5. 本地门禁结果

```text
cargo test -p agent-room-release-manifest -p agent-room-release-tool -p agent-room-desktop --all-features
  Desktop 19、release-manifest 7、release-tool 2 项通过

cargo clippy ... -- -D warnings
  通过

python -m unittest discover -s tools/tests -p "test_*.py"
  97 项通过

pnpm actions:check
  44 个远程 Action 全部固定到完整 SHA

actionlint 1.7.7
  两个新增工作流语法与表达式检查通过

pnpm secrets:check
  720 个文本文件通过
```

核心提交：

- `b415f34`：离线根发布清单与反降级领域验证；
- `c996e82`：断网密钥生成、签名与验证 CLI；
- `7fdd505`：桌面双重签名更新和中断恢复；
- `2db1575`：多镜像产物唯一身份；
- `f82403a`、`87b3da7`、`46700ac`、`54b1046`：候选、聚合、原生和 OCI 逐件证据；
- `55d0b40`、`470187d`：不可跳步晋级和工作流元数据；
- `52f84b1`：候选构建与最终发布工作流。

## 6. 外部门禁

完成状态要求以下证据全部存在：

1. 维护者完成可审计的离线根密钥仪式并配置公开信任根；
2. Windows 与 macOS 候选在 GitHub 受保护环境真实构建；
3. 三个多架构 OCI Index 完成 GHCR 推送、签名和最终复验；
4. 任务 36 封闭测试和任务 40 公网部署通过；
5. 数据库扩展与兼容服务端晋级记录具有真实不可变证据；
6. 测试渠道完成一次正常升级、一次中断恢复和一次显式回滚。

操作步骤见[签名发布 Runbook](../../docs/operations/signed-releases.md)。
