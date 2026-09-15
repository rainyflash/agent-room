# 日常开发与可续跑发布

日常开发使用 `corepack pnpm@10.28.0 desktop:dev`。它启动真实桌面壳及开发前端，修改界面无需生成安装包。提交前运行 `desktop:check`；`desktop:package` 复用同一套原生和浏览器检查，检查失败不会继续生成安装器。`daily-usability.e2e.ts` 已纳入这两个入口。

## 发布入口

`tools/release_flow.py` 组织已有 CI、候选签名、兼容部署、人工/真实宿主验收和公开发行。首次填写参数，此后只需要同一个状态文件。它不自行修改版本、合并代码或延长签名有效期。版本准备、保护分支合并、独立信任公钥和环境审批仍按 [签名发布 Runbook](operations/signed-releases.md) 执行。

完整发布在 Linux 部署主机的已审查、干净工作树运行，需要 Python、Git、已认证的 `gh`、Docker Buildx、Cosign，以及从相同提交构建的 `agent-room-release-tool`。状态目录放在仓库外。下面的路径为运维配置示例，序号与旧版本必须替换成实际已信任值：

```bash
python tools/release_flow.py \
  --state /var/lib/agent-room-release/next/flow.json \
  --repository rainyflash/agent-room \
  --sequence 33 --installed-version 0.1.0-alpha.32 --highest-sequence 32 \
  --trusted-public-key /etc/agent-room/testing-release-public.json \
  --profile full \
  --deploy-config /etc/agent-room/deployment.json \
  --deploy-state /var/lib/agent-room \
  --watch
```

也可使用 `just release-flow` 传同一组参数。恢复时：

```bash
python tools/release_flow.py --state /var/lib/agent-room-release/next/flow.json --watch
```

Windows 可运行候选及客户端发布流程（`--profile client`）；生产镜像部署助手只在 Linux 运行。客户端发布必须提供来自当前候选的数据库及服务器兼容晋级报告，即使服务器无需更换镜像，也不能把“服务健康”直接当成兼容性结论。报告放入状态文件旁的 `candidate` 目录，格式和生成入口见原 Runbook。不要跨机器移动检查点后手工改写其中的路径；首次应选择能完成该发布范围的主机。

## 各阶段做什么

1. 固定版本、提交、序号、发行范围及独立公钥摘要。完整 CI 的八项必需检查全部通过才构建候选。
2. 运行受保护的签名候选工作流，下载完整草稿；验证离线根签名、逐项 SHA、SBOM、Sigstore 来源及提交、OCI 可达性、Tauri 更新签名和精确安装器的原生验收回执。
3. 完整发布先备份并验证备份。备份不重新生成现有配置或密钥。按候选 OCI 摘要执行数据库扩展迁移、身份服务和控制面升级；核对实际镜像 ID、健康、联邦连接及无关服务未被替换，生成兼容晋级证据。
4. 校验首次接入、旧版本升级、真实宿主连续接待三份验收，缺项则停在此处。
5. 运行原有 `public-release` 审批工作流；再次验证证据后公开版本，核对发行资产和 testing 更新渠道都指向相同签名清单。
6. 完整发布最后升级网页镜像，核对运行版本与健康。

退出码 `0` 表示完成，`20` 表示等待远端任务、审批或验收，`1` 表示失败，`130` 表示本地中断。`--watch` 只等待正在运行的远端任务；缺少真实验收或远端失败会退出，不反复制造任务。中断后重跑同一命令即可。

## 失败如何恢复

- 远端运行编号保存在检查点，重试先查询原运行。失败的 CI 可以在 GitHub 重跑失败任务，再执行同一命令。
- 派发超时不等于派发失败。调度器先记录操作号，随后查找同一工作流、同一提交和操作号，不盲目派发第二个候选。可用 `--attach-run ci.yml=运行编号` 明确附加匹配的运行；不匹配会拒绝。
- 若候选已完成签名并归档，只有 Release 上传失败，调度器直接恢复完整归档、重新验证，再补上传缺失资产。已有同名资产须逐字节一致，不覆盖不同内容，不重新签署。
- 已恢复的完整候选会单独保存检查点。即使原归档过期或版本已经公开，网页部署失败后仍可从同一候选续跑；每次启动都会重新验证本地候选签名。公开工作流重跑时也会核对并复用原晋级回执，不改写时间或覆盖证据。
- 没有完整归档时，使用原 Runbook 的 `reuse_run_id` 恢复原生/镜像产物；该情形需人工检查原运行，不能通过修改检查点把失败改成成功。代码或签名候选变更需新版本/序号及独立状态目录。
- Linux 部署保存已验证备份、原容器标识和 `compose.rollback.json`。迁移是 SQLx 记录的幂等扩展迁移。服务升级失败不会发布客户端或网页，也不自动做不可逆数据库回退；需要回滚时使用原 Runbook 的兼容回滚步骤和该目录中的原镜像覆盖文件。
- 重启调度进程会重新检查门禁；同一进程轮询远端任务时复用已通过阶段，避免重复下载/签名校验。状态文件有操作系统排他锁，崩溃后自动释放。

## 真实易用性验收报告

这些验收必须在**当前签名候选**上执行。自动化夹具、开发服务器和无模型协议测试不能当作真实模型回复证据。

| 报告场景               | 必须实际验证                                                                                   |
| ---------------------- | ---------------------------------------------------------------------------------------------- |
| `first-device`         | 首次电脑授权完成、Bridge 保持连接、人物进入房间、回复被接收端验证                              |
| `upgrade`              | 安装旧版后升级当前候选，登录、同一人物身份和未确认投递恢复                                     |
| `continuous-reception` | 真实宿主处理两条不同消息并产生两条已验证回复；空闲不调用模型；手动接管停止后台；交回后保留游标 |

CI 里的 `windows_installer_acceptance.py` 负责无账号的安装、运行中升级、卸载检查；它不能代替上述登录和真实接待验收。每个实际验收执行器或维护者导出一份 JSON：

- `schemaVersion: 1`，`scenario` 为上表值，`version` 和 `revision` 与候选元数据一致。
- `signedManifestSha256` 为此次 `release.signed.json` 的 SHA-256，`capturedAtUnixSeconds` 为实际验收完成时间，不能早于候选生成。
- `result: "passed"`、`fixture: false`；`checks` 的全部必需键见 `tools/release_acceptance.py` 中的 `SCENARIOS`。只将实际观察到的成功项设为 `true`。
- `evidence` 至少一项，包含 `path`、`sha256`、`redacted: true`。文件与报告同目录，命名为 `usability-evidence-场景.扩展名`，保存脱敏的执行日志/截图/回执。报告及这些附件会发布，不能包含授权码、Cookie、令牌或聊天私密正文。
- 连续接待还需 `replyReceipts`，至少两条不同的 `{ "eventId": "$...", "submissionId": "UUIDv7" }`，来自真实发送/接收回执。重复或错误格式会被拒绝。

导入时工具自动计算清单摘要、核对三份报告并复制附件，无需手工编写发布清单：

```bash
python tools/release_acceptance.py --root /var/lib/agent-room-release/next/candidate \
  --assemble /path/to/first-device.json /path/to/upgrade.json /path/to/continuous-reception.json
python tools/release_flow.py --state /var/lib/agent-room-release/next/flow.json --watch
```

验收工具只验证及汇总证据，不替用户输入密码、不伪造宿主调用，也不会为缺失检查填入成功值。
