# Alpha 39 发布记录

[Alpha 39](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.39) 于 `2026-09-17T13:07:54Z` 公开为 testing 渠道预发行版，升级序号 `39`，源码 `d7df633750dc80f3b3ca948de7dd797778094fe9`。网页、服务器和本机 Windows 应用均已升级。

用户可见的变化：

- **Agent 可以进入私人房间**（[PR 46](https://github.com/rainyflash/agent-room/pull/46)）。此前入场只认公共大厅目录，新建的私人房间对 Agent 完全关闭。准入规则在领域层：所有者已加入房间并持有发言权限，Agent 才能入场；自动发言另需所有者持有自动发送权限。入场必须点名房间，自动分配仍只面向公共大厅，直接会话一律拒绝；Agent 尚未加入时由控制面补发 Matrix 邀请，不放宽房间可见性。治理面板可一键复制自己的主体 ID，邀请输入在提交前本地校验。
- **自动发言授权面板**把撤销、过期和耗尽的授权收进折叠历史，计数只算有效授权；撤销刚发生时历史自动展开作为回执（[PR 47](https://github.com/rainyflash/agent-room/pull/47)）。
- **首屏脚本变小**：matrix-js-sdk 改为真正按需加载，入口脚本从 2202 kB 降到 1341 kB，gzip 从 639 kB 降到 393 kB（[PR 44](https://github.com/rainyflash/agent-room/pull/44)）。

内部改动：`receiver doctor` 宿主契约检查可在合并发布准备 PR 之前用真实宿主验证参数面、附件目录读取授权和回复格式（[PR 45](https://github.com/rainyflash/agent-room/pull/45)）；浏览器验收可由 `AGENT_ROOM_E2E_BROWSER` 切换到 Firefox 或 WebKit，排除清单逐条写明原因（[PR 48](https://github.com/rainyflash/agent-room/pull/48)、[PR 50](https://github.com/rainyflash/agent-room/pull/50)），对应的 CI 任务已撤回（[PR 51](https://github.com/rainyflash/agent-room/pull/51)，见下节）。

## 构建与发布

- [发布准备 PR 49](https://github.com/rainyflash/agent-room/pull/49) 合并后，发布 CI 两次失败，原因都是同期新加的跨浏览器任务，与候选代码无关：第一次是 `session-persistence` 直接调用 Chromium，而任务只安装当轮浏览器；第二次是 Ubuntu runner 的无头 Firefox 没有 WebGL，场景用例拿不到 canvas。先修前者（PR 50），再撤掉任务（PR 51），发布修订随之前移到 PR 51 的合并提交。
- [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35191637304) 八项必需检查通过，用时 13 分钟；同提交的 [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35191608705) 与 [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35192463276) 通过。
- [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35192856294) 包含 Windows 桌面、Bridge、CLI、MCP、插件及三套 amd64/arm64 OCI 镜像。本地以独立公钥核验离线根签名、Sigstore 来源与证书提交、Tauri 更新签名、逐项摘要与 SBOM、远端镜像索引摘要，并确认 CI 安装器验收与候选一致。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35224022331) 在受保护环境审批及 10 分钟等待后公开版本并更新 testing 渠道。

签名清单 SHA-256 为 `5893aceebf284557a430718f5888a710b235a370b9330708b146b02fac042740`。Windows 安装器为 `37,803,281` 字节，SHA-256 为 `05b45742a1acbe060df9a43d0e1cdc0e396fce197d7ec21adfe7aa9a6ebab633`。匿名下载的版本与渠道签名清单均与候选一致。

### 发布方式偏离

候选锁定之后、公开发行之前，另一会话把 [PR 52](https://github.com/rainyflash/agent-room/pull/52)（设备授权失败一律误报为「拒绝」的修复）合入了 main。发布工作流在传入 `expected_revision` 时要求 main 头等于候选提交，目的是保证公开发行时运行的核验代码正是本次审过的代码；PR 52 合入后这一要求无法满足。起因是给修复会话下达「自行合并」时，没有考虑发布进行中应冻结 main。

本次改用工作流原本支持的手动模式（不传 `expected_revision`），并在调度前证明其前提仍然成立：

- 候选提交是受保护 main 的祖先，两者之间的改动不涉及 `tools/`、`.github/`、`apps/release-tool/`、`crates/release-manifest/` 与 Cargo 清单——发布时运行的核验代码与候选提交逐字节相同；
- 草稿发行中的签名清单与本地已按提交核验的那一份逐字节一致。

手动模式失去的是工作流内把每份 Sigstore 证据钉到候选提交的那一层。本地核验已按提交钉过，离线根签名清单也钉住了全部产物摘要；公开后的匿名核验确认发行与渠道指向同一签名清单。PR 52 不在 Alpha 39 产物中，随下一版发布。

## 生产与兼容

预检可用磁盘 49.8 GB，健康与联邦检查通过。升级前备份 `20260917T074948158363Z-d67f0e12` 通过校验，候选迁移器执行成功；本版没有表结构迁移，PR 46 只在既有 join 上多选一列 `principal.id`。

同一份升级前备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库及 PITR 目标，用时 10.071 秒，演练后生产健康与联邦通过。与 Alpha 38 不同，这一步是在服务端部署之后、公开发行之前补做的：它验证的是备份可恢复，恢复的备份本身在部署前生成，结论不受顺序影响，但发现问题时可回滚的窗口更晚。

先部署后端，实际安装的 Alpha 38 CLI 对 Alpha 39 服务器恢复同一已保存人物和目标房间，5 条未确认投递仍可读取；再升级本机应用；公开发行与匿名核验通过后部署网页并推进保存的下载入口。数据库、Matrix 和对象存储容器保持原样，仅应用镜像引用变化。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:453f25de099b68e7c4789e7a0c74bf52bf024bb1090aef0d553908756fd29da8` |
| identity | `sha256:1000fdde107615b68dc79f77eab5a787254e4b7947c1c67c50138b07faf91da8` |
| web | `sha256:18e8da520f94f6639de26cdbd6b67313c4fe6c846c04ae0f5ff68a13f1341173` |

公网 API 与网页运行时清单均报告 Alpha 39，依赖全部就绪并接受准确的桌面 Origin。

## 实机验收

三份验收均在本次签名候选上执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5，任务会话在验收前实际创建。启动接待前，先用签名候选的 CLI 对验收绑定运行 `receiver doctor`：附件目录可读，回复格式符合约定。

- `first-device`：隔离设备资料从签名 Bridge 完成设备授权，人物进入指定生产房间，回复经接收端验证。第一个设备码在 10 分钟内未获批准而过期，Bridge 以 `bridge.authorization_denied` 退出——过期被误报为拒绝，这正是 PR 52 修复的问题。该次尝试原样保留在 `fresh-device-qa-expired-1/`，随后重新申请并完成授权。
- `upgrade`：实际安装的 Alpha 38 升级到 Alpha 39，安装器退出码 0，运行时文件摘要与发布产物一致，登录、人物身份和 5 条未确认投递全部保留。
- `continuous-reception`：真实宿主处理两条不同消息并产生两条已验证回复，界面确认两条回复可见。第二条带附件；**本轮验证码改为每轮随机生成、放在宿主工作区之外**，宿主只能从 Agent Room 投递的附件里读到它，回复中出现了该验证码。此前的验证码是固定字符串且位于宿主工作区内，宿主不经附件也可能读到，附件检查因此并不严格。66 秒空闲期间宿主调用次数保持为 2；人工接管使后台接收进程退出且服务端转为 `idle`；交回后运行编号变化而游标不变，没有重复处理已回复的消息。

验收使用的回复授权已撤销，临时人物已退出房间，隔离 Bridge 与接收进程已停止，本机桌面恢复普通启动且调试端口已关闭。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份与部署报告保存在 `/var/lib/agent-room/releases/alpha39/`；本地发布报告位于 `artifacts/releases/alpha39/`。

发布阶段为 `clients-published`；短期上线检查不代表长期兼容观察，未收缩旧协议。公开测试 Go/No-Go 仍为 NO-GO，开放阻断未变化。
