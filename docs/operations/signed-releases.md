# 签名发布与安全更新 Runbook

## 1. 信任边界

Agent Room Alpha 使用三层彼此独立的证据，任何一层都不能替代另一层：

1. GitHub OIDC + Sigstore 证明二进制、插件、更新 JSON 和 OCI 镜像由受保护的候选工作流生成；
2. Tauri minisign 密钥为桌面更新归档提供安装器强制验证；
3. testing 专用 Ed25519 发布密钥签署根发布清单，约束渠道、单调序号、版本、有效期、回滚来源和全部产物摘要。

OS 代码签名是第四个可选层，不属于更新信任根。没有商业代码签名不会关闭以上任一验证。

testing 发布私钥和 Tauri 私钥是两把独立的在线密钥，只能存放在受保护的 `release-candidate` Environment，不能进入 Git、构建产物、日志或容器镜像。testing 发布私钥泄露会允许攻击者伪造 Alpha 发布清单，因此该信任模型明确不用于 stable。Tauri 私钥泄露仍不能单独伪造 testing 根清单，反之亦然。

## 2. testing 密钥初始化

从已审查提交构建签名工具，在受控临时目录生成 testing 专用密钥：

```bash
cargo build --locked --release --package agent-room-release-tool
target/release/agent-room-release-tool keygen \
  --private-key /tmp/agent-room-testing-release-private.json \
  --public-key /tmp/agent-room-testing-release-public.json
```

把私钥 JSON 原样写入 `release-candidate` Environment Secret `AGENT_ROOM_TESTING_RELEASE_PRIVATE_KEY`。把公开文档中的 `keyId` 和 `publicKey` 分别写入仓库变量 `AGENT_ROOM_TESTING_RELEASE_KEY_ID` 与 `AGENT_ROOM_TESTING_RELEASE_PUBLIC_KEY`。核对变量后删除临时私钥文件，不保留开发机副本。

其余仓库变量为：

- `AGENT_ROOM_TAURI_UPDATER_PUBLIC_KEY`；
- `AGENT_ROOM_RELEASE_STABLE_URL`，固定为 `https://github.com/OWNER/REPO/releases/download/channel-stable/release.signed.json`；
- `AGENT_ROOM_RELEASE_TESTING_URL`，固定为 `https://github.com/OWNER/REPO/releases/download/channel-testing/release.signed.json`。

受保护环境 `release-candidate` 保存 testing 发布私钥、`TAURI_SIGNING_PRIVATE_KEY` 和可选密码。Alpha 允许单维护者在 `public-release` 环境自审批；这只是误操作门槛，不冒充双人安全复核。stable 上线前必须引入不同的离线根密钥、独立审批与客户端信任迁移，绝不能把 testing 私钥直接升级成 stable 根。

## 3. 候选构建

只从受保护 `main` 分支手动运行 `签名发布候选`：

- `tag` 必须与 `Cargo.toml` workspace 版本完全一致；
- `sequence` 必须高于该渠道历史最高值；
- `rollback_from` 只在退回精确已安装版本时填写；
- `profile=client` 是日常 Alpha 默认值，只交付 Windows 客户端运行时；
- `profile=full` 只用于服务端也需要发布时，额外生成三套双架构 OCI 镜像；
- Alpha 工作流只接受 `testing`；未来 `stable` 使用独立密钥与序号。

工作流执行以下动作：

1. 构建 Windows x64 的 NSIS 安装器、Tauri 更新归档、Bridge、通用 MCP 和 Codex 配置适配器；macOS ARM64 只在维护者自托管 runner 上单独手动验收，不进入首发候选；
2. 仅当 `profile=full` 时，在 GitHub 官方 amd64 与 arm64 Linux runner 上分别原生构建 `control-plane`、`identity`、`web`，记录每个平台不可变 digest，再合并为 amd64/arm64 OCI Index；禁止在 x64 runner 上用 QEMU 编译 Rust 控制面；
3. 为每个产物生成 CycloneDX SBOM、摘要和 Sigstore bundle；OCI bundle 签署原始 Index manifest，并要求该文件 SHA-256 与远端不可变 digest 完全一致；
4. 合并 Tauri 平台更新清单并同样生成 SBOM 与签名；
5. 生成 `release.json`、`release-evidence.json` 和连续晋级记录；
6. 从受保护 Environment 注入 testing 私钥，签署 `release.signed.json`，再用仓库公钥反向验证；
7. 立即删除 runner 临时私钥文件，并创建只有维护者可见的 GitHub Draft Release。

GitHub runner 是短生命周期环境，但它仍是在线信任边界。Environment Secret、分支保护、固定 Action SHA 和审计日志缺一不可。

## 4. 数据库与服务端晋级

发布记录严格按以下顺序推进，工具拒绝跳步：

```bash
python tools/release_promotion.py advance \
  --record release-promotion.json \
  --output release-promotion-database-expanded.json \
  --stage database-expanded \
  --evidence-url https://evidence.example/releases/v0.2.0/database-expand.json \
  --evidence-sha256 <64-hex> \
  --recorded-at-unix-seconds <unix>

python tools/release_promotion.py advance \
  --record release-promotion-database-expanded.json \
  --output release-promotion-compatible-server.json \
  --stage compatible-server \
  --evidence-url https://evidence.example/releases/v0.2.0/server.json \
  --evidence-sha256 <64-hex> \
  --recorded-at-unix-seconds <unix>
```

两个证据必须来自不可变、受访问控制的真实部署报告。报告必须使用 `agent-room.release-deployment-evidence` Schema，绑定候选版本、完整 Git SHA、阶段、采集时间和全部通过的检查。把报告与最终晋级文件上传到同一 Draft Release；最终工作流会从晋级 URL 定位同一 Release 的本地资产、复算 SHA-256，并拒绝缺失、篡改、失败或跨版本证据。客户端发布后进入 `clients-published`；观察错误率和协议兼容比例后推进 `compatibility-observed`；只有下一发布窗口才允许推进 `legacy-contracted` 并删除旧路径。

## 5. stable 的未来离线签名边界

Alpha 不执行这一步。stable 发布启用前，必须在断网、全盘加密设备上生成独立根密钥，并让已发布客户端通过受信升级接收 stable 公钥。未来 stable 离线签名命令形态为：

```bash
target/release/agent-room-release-tool sign \
  --private-key /media/offline/agent-room-stable-release-private.json \
  --manifest /media/transfer/release.json \
  --output /media/transfer/release.signed.json
```

签名工具拒绝覆盖文件、校验清单静态策略，并在使用后清零内存中的私钥缓冲。stable 私钥不得进入 GitHub Secrets；这个约束不能反向套用到明确降级的 testing Alpha 策略上。

## 6. 最终发布

从 `main` 手动运行 `验证并发布 testing 签名发行`，提供当前渠道已安装版本和最高可信序号。工作流将：

- 验证 testing Ed25519 签名、渠道、有效期、序号和显式回滚；
- 重新计算所有本地文件摘要并验证逐件 SBOM；
- 使用已修补并显式锁定的 Cosign 3.1.3，以精确 GitHub Workflow OIDC 身份验证所有本地 Sigstore bundle；
- 对 `full` 候选的三个 OCI 产物复核“已签 manifest 摘要 = 发布 URL digest”，再从 GHCR 探测该精确 digest 可达，拒绝可变标签、摘要错配或已删除镜像；`client` 候选不包含 OCI 产物；
- 要求晋级记录恰好处于 `compatible-server`；
- 以 prerelease 公开版本，再更新 `channel-testing` 的签名根清单；
- 追加并上传 `clients-published` 晋级证据。

任一步失败都不会推进客户端本地可信序号。桌面端先写入 pending 安装记录，只有目标版本真正启动后才提交序号；下载中断、进程终止或安装失败仍可重试。客户端拒绝过期清单、篡改包、重复序号、跨渠道清单和未指明来源版本的降级。

## 7. MCP、宿主适配器与 Bridge 兼容

IPC 握手必须完整匹配协议版本。Bridge 或 MCP 返回 `bridge.ipc.version_incompatible` 时：

- 通用 MCP 只显示“MCP 与 Bridge 必须升级到同一发行版本”；
- Desktop Supervisor 进入停止/升级态；
- MCP 不注册残缺工具，不允许部分能力继续工作。

Codex、Claude Code 与 Cursor 适配器只配置通用 MCP 路径，不参与 IPC 协商。恢复方式是重新安装同一 Release 的 Windows 安装器；独立二进制只用于核验和高级集成，不允许复制不同版本拼装 Runtime。

## 8. 故障与撤回

- 候选失败：保留 Draft 与 Actions 证据，不更新渠道入口；修复后使用新序号和新标签重新候选。
- 公布后发现客户端缺陷：生成更高序号、`rollbackFrom` 精确等于当前版本的回滚清单；不得移动旧标签或降低序号。
- 渠道入口更新失败：公开资产仍不可被自动更新发现；修复入口后重试最终步骤。
- Tauri 密钥泄露：冻结 testing 渠道，轮换 Tauri 密钥，并通过仍可信的 testing 发布密钥发布新客户端。
- testing 发布密钥疑似泄露：立即冻结 Alpha 发布；不能用同一密钥“自我声明安全”，必须带外分发携带新 testing 公钥的客户端并公开事件报告。
- stable 离线根疑似泄露：立即冻结 stable；需要带外分发新根客户端，不能回退复用 testing 密钥。

公开前仍必须完成封闭测试、真实公网部署和外部安全评审。工作流存在不等于这些运行证据已经通过。
