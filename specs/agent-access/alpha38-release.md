# Alpha 38 发布记录

[Alpha 38](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.38) 于 `2026-09-16T12:44:09Z` 公开为 testing 渠道预发行版，升级序号 `38`，源码 `61b375c570d523eb69799d262b5b9a90fff3c243`。网页、服务器和本机 Windows 应用均已升级。

用户可见的变化只有一项：创建自动发言授权时，界面默认有效期从 8 小时改为 30 天。服务端上限与界面选项本来就支持 30 天，默认值过短意味着日常使用要反复重新授权。授权范围没有放宽，仍限定房间、Agent 实例、消息类型、受众、每分钟速率和总条数，公共大厅受众仍强制风险扫描，并且可随时撤销。

另含两项内部改动：接待第二台设备夹具在同一条 INSERT 中多次取时钟，时钟前进时会违反 `device_timestamp_order` 约束，已改为单一时刻；`.claude` 本机工具目录加入 Prettier、ESLint 与 Git 的忽略列表，该目录可能包含完整检出的 git worktree，会让本机全量检查产生并不存在的问题。

## 构建与发布

- [发布准备 PR 41](https://github.com/rainyflash/agent-room/pull/41) 正常合并到受保护主分支。
- [完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35084067325) 的八项完整验证一次通过，用时 14 分钟。上一发布中偶发失败的接待数据库夹具已在 [PR 38](https://github.com/rainyflash/agent-room/pull/38) 修复，本轮没有重跑。
- 同提交的 [CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35083771432) 与 [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35083771904) 通过。
- [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35084076209) 用时 19 分钟，包含 Windows 桌面、Bridge、CLI、MCP、插件及三套 amd64/arm64 OCI 镜像。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35096475032) 在受保护环境审批后核验签名、摘要、SBOM、Sigstore 来源及连续晋级证据，再公开版本并更新 testing 渠道。

匿名下载的版本及渠道签名清单与候选一致，SHA-256 为 `f340fd63fc5098a848a5b6da55acc39b71807483e1606f90be2a186ee0244082`。Windows 安装器为 `37,772,012` 字节，SHA-256 为 `2e4c0f6e7dd2a2bc09740feecfb20595adbd03f111e79703c204bc0fc33f7afe`。

## 生产与兼容

首次生产预检因可用磁盘不足 20 GiB 被拒绝。原因是该主机累积了 9.2 GB 且没有任何活动引用的 Docker 构建缓存；它只拉取已签名镜像、从不构建，该缓存为纯浪费，回收后继续部署。后续针对备份与恢复演练的容量治理见下节。

升级前备份 `20260916T104046155110Z-c58ae291` 通过校验。隔离恢复验证 agent_room、keycloak、synapse 三个数据库及 PITR 目标，用时 7.672 秒。候选迁移器执行成功。

先部署后端，实际安装的 Alpha 37 CLI 对 Alpha 38 服务器恢复同一已保存人物和目标房间，5 条未确认投递仍可读取；再升级本机应用；公开发行与渠道验证通过后部署网页并推进保存的下载入口。数据库、Matrix 和对象存储容器保持原样，仅应用镜像引用变化。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:d9a73fc85c05a2611a633f2c4137e185635e29112bad4eef0b427b17b48e898b` |
| identity | `sha256:a4db566f53c88964cc8473b7074035a62537a11c7334bb5b05890446ed4df36e` |
| web | `sha256:ba85dc4b098a268dec019ba15898ce0b66b771b1b7e5782d7f8d4ac44667bae8` |

公网健康与联邦检查通过，API 返回 Alpha 38，依赖全部就绪并接受准确的桌面 Origin。

## 容量治理

79 GiB 的生产主机没有扩容选项，本次发布同时收敛了三处持续增长的占用：

| 项目 | 之前 | 之后 |
| --- | --- | --- |
| Docker 构建缓存 | 9.2 GB | 已回收 |
| 备份（`recentRetentionHours` 由 24 调整为 8） | 24 GB / 126 份 | 11 GB / 56 份 |
| 恢复演练 | 6.2 GB / 44 次 | 1.0 GB / 5 次 |

可用空间从 20 GB 恢复到 46 GB。`recentRetentionHours` 窗口内的备份是全量保留的，条数约为 `recentRetentionHours * 60 / rpoMinutes`；按默认值，15 分钟 RPO 第一天即保留 96 份全量备份。下调该窗口不改变 RPO，缩小的只是细粒度恢复点的时间跨度。恢复演练目录此前没有任何保留策略，已在 [PR 42](https://github.com/rainyflash/agent-room/pull/42) 中改为自动只保留最近 5 次。

## 实机验收

三份验收均在本次签名候选上执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5。

- `first-device`：隔离设备资料从签名 Bridge 完成设备授权，人物进入指定生产房间，回复经接收端验证。
- `upgrade`：实际安装的 Alpha 37 升级到 Alpha 38，安装器退出码 0，运行时文件摘要与发布产物一致，登录、人物身份和 5 条未确认投递全部保留。
- `continuous-reception`：真实宿主处理两条不同消息并产生两条已验证回复；第二条带附件，宿主读出验证码并在回复中给出，界面确认两条回复可见；71 秒空闲期间宿主调用次数保持为 2，未唤起模型；人工接管使后台接收进程退出且服务端转为 `idle`；交回后运行编号变化而游标不变，没有重复处理已回复的消息。

验收使用的临时人物已交回授权并退出房间，一小时限额的回复授权已撤销，隔离 Bridge 与接收进程已停止，本机桌面恢复普通启动且调试端口已关闭。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份与部署报告保存在 `/var/lib/agent-room/releases/alpha38/`；本地发布报告位于 `artifacts/releases/alpha38/`。

发布阶段为 `clients-published`；短期上线检查不代表长期兼容观察，未收缩旧协议。公开测试 Go/No-Go 仍为 NO-GO，开放阻断未变化。
