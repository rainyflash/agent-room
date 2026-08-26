# 签名发布与安全更新 Runbook

## 1. 信任边界

Agent Room 使用三层彼此独立的证据，任何一层都不能替代另一层：

1. GitHub OIDC + Sigstore 证明二进制、插件、更新 JSON 和 OCI 镜像由受保护的候选工作流生成；
2. Tauri minisign 密钥为桌面更新归档提供安装器强制验证；
3. 断网保存的 Ed25519 发布密钥签署根发布清单，约束渠道、单调序号、版本、有效期、回滚来源和全部产物摘要。

OS 代码签名是第四个可选层，不属于更新信任根。没有商业代码签名不会关闭以上任一验证。

根私钥不得进入 GitHub Secrets、开发机同步盘、密码管理器浏览器扩展或容器镜像。GitHub 只保存公开的 Key ID 与公钥。Tauri 私钥是独立的在线制品密钥，只能存放在受保护的 `release-candidate` Environment；它泄露时不能伪造离线根清单。

## 2. 一次性密钥仪式

在断网、全盘加密且完成系统更新的专用设备上，从已审查提交构建签名工具：

```bash
cargo build --locked --release --package agent-room-release-tool
target/release/agent-room-release-tool keygen \
  --private-key /media/offline/agent-room-release-private.json \
  --public-key /media/transfer/agent-room-release-public.json
```

至少两名维护者核对工具源码、提交 SHA、二进制摘要和生成的 Key ID。私钥制作两个加密离线副本并分别保管；传输介质只带公开文档离开离线区。

将公开文档中的 `keyId` 和 `publicKey` 分别配置为仓库变量：

- `AGENT_ROOM_RELEASE_KEY_ID`；
- `AGENT_ROOM_RELEASE_PUBLIC_KEY`；
- `AGENT_ROOM_TAURI_UPDATER_PUBLIC_KEY`；
- `AGENT_ROOM_RELEASE_STABLE_URL`，固定为 `https://github.com/OWNER/REPO/releases/download/channel-stable/release.signed.json`；
- `AGENT_ROOM_RELEASE_TESTING_URL`，固定为 `https://github.com/OWNER/REPO/releases/download/channel-testing/release.signed.json`。

受保护环境 `release-candidate` 保存 `TAURI_SIGNING_PRIVATE_KEY` 和可选密码；`public-release` 至少要求不同维护者审批。公开根密钥轮换必须先由旧根签署一个携带新公钥客户端的正常升级，等待兼容比例达标后才能使用新根签下一序号。

## 3. 候选构建

只从受保护 `main` 分支手动运行 `签名发布候选`：

- `tag` 必须与 `Cargo.toml` workspace 版本完全一致；
- `sequence` 必须高于该渠道历史最高值；
- `rollback_from` 只在退回精确已安装版本时填写；
- `stable` 与 `testing` 拥有互相隔离的可信序号。

工作流执行以下动作：

1. 构建 Windows x64、macOS x64 的 Tauri 更新归档、Bridge 和平台插件；
2. 构建 `control-plane`、`identity`、`web` 的 amd64/arm64 OCI Index；
3. 为每个产物生成 CycloneDX SBOM、摘要和 Sigstore bundle；
4. 合并 Tauri 平台更新清单并同样生成 SBOM 与签名；
5. 生成 `release.json`、`release-evidence.json` 和连续晋级记录；
6. 创建只有维护者可见的 GitHub Draft Release。

候选工作流没有根私钥，因此无法自行把草稿变成受客户端信任的更新。

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

两个证据必须来自不可变、受访问控制的真实部署报告。把最终文件上传到同一 Draft Release。客户端发布后进入 `clients-published`；观察错误率和协议兼容比例后推进 `compatibility-observed`；只有下一发布窗口才允许推进 `legacy-contracted` 并删除旧路径。

## 5. 离线签名

在线审查机先验证候选摘要、SBOM、Sigstore 身份和晋级证据，再把唯一的 `release.json` 通过只读介质带入离线区。离线机不连接 GitHub：

```bash
target/release/agent-room-release-tool sign \
  --private-key /media/offline/agent-room-release-private.json \
  --manifest /media/transfer/release.json \
  --output /media/transfer/release.signed.json
```

签名工具拒绝覆盖文件、校验清单静态策略，并在使用后清零内存中的私钥缓冲。将 `release.signed.json` 上传到 Draft Release；不要上传私钥、离线日志或介质目录清单。

## 6. 最终发布

从 `main` 手动运行 `验证并发布离线签名发行`，提供当前渠道已安装版本和最高可信序号。工作流将：

- 验证离线 Ed25519 签名、渠道、有效期、序号和显式回滚；
- 重新计算所有本地文件摘要并验证逐件 SBOM；
- 用精确 GitHub Workflow OIDC 身份验证所有 Sigstore bundle；
- 远程验证三个不可变 OCI digest 的签名；
- 要求晋级记录恰好处于 `compatible-server`；
- 公开版本，再更新 `channel-stable` 或 `channel-testing` 的签名根清单；
- 追加并上传 `clients-published` 晋级证据。

任一步失败都不会推进客户端本地可信序号。桌面端先写入 pending 安装记录，只有目标版本真正启动后才提交序号；下载中断、进程终止或安装失败仍可重试。客户端拒绝过期清单、篡改包、重复序号、跨渠道清单和未指明来源版本的降级。

## 7. 插件与 Bridge 兼容

IPC 握手必须完整匹配协议版本。Bridge 或 MCP 返回 `bridge.ipc.version_incompatible` 时：

- Codex 插件只显示“插件与 Bridge 必须升级到同一发行版本”；
- Desktop Supervisor 进入停止/升级态；
- MCP 不注册残缺工具，不允许部分能力继续工作。

恢复方式是安装同一 Release 中、同一平台的 Bridge 与 Codex 插件，不允许复制单个二进制拼装版本。

## 8. 故障与撤回

- 候选失败：保留 Draft 与 Actions 证据，不更新渠道入口；修复后使用新序号和新标签重新候选。
- 公布后发现客户端缺陷：生成更高序号、`rollbackFrom` 精确等于当前版本的回滚清单；不得移动旧标签或降低序号。
- 渠道入口更新失败：公开资产仍不可被自动更新发现；修复入口后重试最终步骤。
- Tauri 密钥泄露：冻结两个渠道，轮换 Tauri 密钥并通过仍可信的离线根发布新客户端。
- 离线根疑似泄露：立即冻结发布，不能用同一信任根“自我声明安全”；需要带外分发新根客户端并公开事件报告。

公开前仍必须完成封闭测试、真实公网部署和外部安全评审。工作流存在不等于这些运行证据已经通过。
