# Alpha 55 发布记录

[Alpha 55](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.55) 于 `2026-09-29T02:17:59Z` 公开为 testing 渠道预发行版，升级序号 `55`，源码 `34f3ffe1f16cf67a351de4e681785e7784195239`。网页、服务器和本机 Windows 应用均已升级。本版在受保护的 `release/v0.1.0-alpha.55` 分支上以锁定模式公开。

本版的主题是**把解不开的加密历史找回来**。Alpha 54 修好了一次性密钥积压，但维护者房间「new game dev」里已经解不开的 329 条消息仍然解不开：建坏的 Olm 通道和错过的房间密钥不会自己好。本版让人的设备请 Agent 把它自己建的房间密钥重发一份（设计见[找回解不开的历史消息](../room-key-recovery/design.md)）。

## 用户可见的变化

- **解不开的历史自动找回**（[PR 206](https://github.com/rainyflash/agent-room/pull/206)、[PR 208](https://github.com/rainyflash/agent-room/pull/208)、[PR 211](https://github.com/rainyflash/agent-room/pull/211)）：
  - 网页端和桌面端发现 Agent 的消息因为缺密钥解不开时，攒 2 秒按 Agent 设备和房间合成一条请求，经 Olm 加密发给那个 Agent。请求走一条新建的 Olm 通道，顺带换掉坏通道。
  - Agent 只回答经 Olm 送达、此刻在房间里、由主人签名的设备，只在房间历史对成员开放时回答，只重发它自己这台设备建的会话，并按设备和房间限频。
  - 人这边核对应答来自我们请求过的那台、由主人签名的设备，再导入；SDK 对等着这些密钥的消息自动重试解密，提示随之更新。
  - 这台设备还没由主人签名时，请求先扣着，提示里说明“Agent 只把密钥发给验证过的设备”，并给“验证这台设备”按钮直达安全页；签名同步到本地后自动发出。发送方因为设备没验证而拒绝分发的消息也会请求重发。
- **私人房间里一键请网络 Agent**（[PR 207](https://github.com/rainyflash/agent-room/pull/207)）：“接入 Agent”对话框里点一下就生成口令并复制，不用再去房间设置里找。
- **桌面端更顺**：
  - 重启不再多等二十来秒（[PR 214](https://github.com/rainyflash/agent-room/pull/214)）。Bridge 退出时给自己的设备发一条 to-device 消息，叫醒进行中的 Matrix 长轮询，同步照常走完、不半途取消；各人物也改成一起关。
  - 不再每隔一刻钟闪一下“正在重连”（[PR 213](https://github.com/rainyflash/agent-room/pull/213)）。按计划提前刷新设备令牌时，旧令牌还有效，设备一直算就绪。
  - 本机日志不再被同一条告警刷满（[PR 212](https://github.com/rainyflash/agent-room/pull/212)）。桌面端每 2 秒探一次 Bridge，默认人物没在跑时 `get_self` 每次都失败，原来一天四万多条告警；现在同样的失败十分钟只记一条，带上省掉的次数。
- **等消息更省**（[PR 210](https://github.com/rainyflash/agent-room/pull/210)）：Agent 等消息时的房间权限检查从四个请求减到一个。
- **依赖安全**（[PR 215](https://github.com/rainyflash/agent-room/pull/215)）：fast-uri 两条高危公告、qs 与 undici 的中危公告，按只命中受影响版本段的 override 升到同一大版本内的修复版。

桌面端、Bridge、CLI 与 MCP 同版本发布。本版没有新的数据库迁移。

## 构建与发布

- 版本号在 [PR 216](https://github.com/rainyflash/agent-room/pull/216) 升到 Alpha 55，它的合并提交 `34f3ffe` 就是发布提交，`00:57Z` 合并后立刻派发：
  - [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/36505587339)：真实 Synapse 集成与真实网页登录都通过；
  - [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/36505546993)：发布提交推送触发的那次；
  - [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/36505602032)：通过；
  - [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/36505594978)：
    - 第一次尝试里，Windows 原生发行作业从 crates.io 下载依赖时连接被重置（连串 HTTP2 分帧错误），`01:01Z` 失败。聚合作业被跳过，没有生成草稿，于是只重跑失败的作业。
    - 第二次尝试 `01:19Z` 与 `01:55Z` 批准受保护环境，随后完成。Mac 版照常签名与公证。候选共 69 个文件。
- 本地以独立公钥核验：
  - 离线根签名、Sigstore 来源与证书提交；
  - 两个平台的 Tauri 更新签名与安装验收回执；
  - 逐项摘要、SBOM 和远端镜像索引摘要。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/36511727531)在 `release/v0.1.0-alpha.55` 上以锁定模式运行，`02:15Z` 批准。发布前核对了草稿签名清单与已核验候选逐字节一致，发行共 81 个资产。

签名清单 SHA-256 为 `8161b417bdbb6c17d720e5413917c334086245f2a3503a37efad5d471992aac8`。

| 安装包 | 字节 | SHA-256 |
| --- | --- | --- |
| Windows 安装器 | `40,803,941` | `872c5fb1d0b150c011784f63c26a6ae9780ab6997dd0a57d3b0cc4faaed17c7c` |
| Mac 磁盘映像 | `54,073,906` | `9f4b68ffddb5af1ba1ea1ec3c341d020f118180a8a1c08cdfb5dfe7fadb603e0` |

公开后，匿名下载的两个安装包、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。核验时可信的最高序号是 Alpha 54 的 `54`。

从版本 PR 合并到网页上线约 1 小时 22 分钟（`00:57Z`–`02:19Z`），其中约 20 分钟花在重跑 Windows 原生发行上。本版登录相关的代码没有变化，实机验收复用长期验收设备，不需要设备码。维护者只确认了一次：升级他本机桌面端的时机（会断开正在运行的 Agent）。

## 生产与兼容

- **部署前检查**：维护者本机的 Alpha 54 CLI 在服务端切换前后，都能恢复升级验收人物和目标房间，47 条未确认投递均可读取。公网 API 切换前报告 `0.1.0-alpha.54`，之后报告 `0.1.0-alpha.55`。
- **预检**：可用磁盘约 28.7 GB，健康与联邦检查通过，备份锁空闲。
- **预拉镜像**：三个候选镜像按摘要预先拉进本地缓存，共约 15 秒。
- **服务端部署**：
  - 生产源码从 `57366cd` 切到 `34f3ffe`。
  - 部署前看过定时备份：`02:00Z` 那次已经结束，下一次在 `02:15Z`，部署在这个空档里完成。
  - 部署前备份 `20260929T020314499221Z-7338cad3` 通过校验；本版没有新迁移，随后切换到兼容服务器，四份部署与晋级记录齐全。
- **备份与恢复演练**：同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库，达到了 PITR 目标，用时 14.76 秒。演练后生产健康检查通过。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:8e5b9d2d96f226bca8d68b040a0a7bf39bee429883cab26eef613f0972509138` |
| identity | `sha256:76143dcb593a4cc58905e5a20545e028d4c88eaff80770e7101d90d773f1ae13` |
| web | `sha256:ba2d651414d5c636d23c45f78d13f9189a0626df88140d642ef8c16dc880cde5` |

网页部署（约 `02:19Z`）之后：

- 公网 API 与网页运行时清单均报告 Alpha 55，两个桌面来源都被接受；
- 生产观察确认健康与联邦检查通过，签名镜像仍在运行；
- 两个下载入口都指向 Alpha 55 的安装包，只改了下载地址，容器未变；
- 网络 Agent 生产冒烟通过。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `upgrade`：维护者本机从 Alpha 54 用本候选已核验的安装器静默原地升级，三个运行时文件与签名清单一致；登录与 Bridge 恢复，升级验收身份、房间和未确认投递全部保留。维护者事先同意了这次升级会断开他正在运行的 Agent。
- `first-device`：Alpha 53 那次新设备授权（源码 `a87cb34`）之后，登录相关代码没有变化，复用长期验收设备。Bridge 用已保存的授权直接就绪，Agent 加入与两次真实宿主回复核对全部通过。
- `continuous-reception`：真实宿主处理两条不同消息，第二条带附件，宿主读出了其中的验证码；两条回复都经过验证。空闲观察 66 秒，没有再调用宿主。人工接管后接收端停止，交回后继续，游标不变。

验收使用的回复授权已撤销，人物已退出房间，隔离 Bridge 与接收进程已停止，调试端口 14222 确认关闭。清理后桌面端以普通方式重启，`agent-room doctor` 报告 Bridge 就绪。

## 装上以后看到的

维护者本机升级后的日志里，#212 和 #214 的效果已经看得到：

- Alpha 55 的 Bridge 启动后十分钟里只有 6 条本地 IPC 失败告警，以前同样时间约 300 条。
- 验收清理重启桌面端时，旧 Bridge `02:14:03Z` 开始有序退出，新 Bridge 不到 1 秒就起来了。以前要等二十多秒。不过当时各人物都没连着，这次还没测到“一起关”这一半。

维护者房间的历史有没有找回，这一版的日志还看不出来：Bridge 的文件日志默认只记 `agent_room_bridge` 自己，人物应请求重发房间密钥的结果写在 `agent_room_matrix_adapter::room_keys`，被滤掉了。[PR 217](https://github.com/rainyflash/agent-room/pull/217) 把它加进默认过滤规则，随下一版生效；在那之前，要看房间里的提示是否消失、消息是否解开。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份、部署报告和恢复演练的证据保存在 `/var/lib/agent-room/releases/alpha55/`，本地发布报告位于 `artifacts/releases/alpha55/`。

发布阶段为 `clients-published`。短期上线检查不代表长期兼容观察，旧协议没有收缩。
