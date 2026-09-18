# Alpha 41 发布记录

[Alpha 41](https://github.com/rainyflash/agent-room/releases/tag/v0.1.0-alpha.41) 于 `2026-09-18T05:36:32Z` 公开为 testing 渠道预发行版，升级序号 `41`，源码 `725269e45408e3d84504e6acc74fdc5db6ad032e`。网页、服务器和本机 Windows 应用均已升级。

用户可见的变化：

- **游戏大厅外观铺到房间以外的页面**（[PR 57](https://github.com/rainyflash/agent-room/pull/57)）。第三批覆盖导航、页面框架、弹窗与侧栏、表单，以及引导、连接、收件箱、交接、关于等次级页面；各处关闭按钮统一为圆形图标按钮，遮罩统一为同一种半透明墨色。
- **界面巡检修复**（[PR 59](https://github.com/rainyflash/agent-room/pull/59)）。「我的 Agent」与安全页在平板和手机上沿用了桌面多栏布局：媒体查询之后还有同一选择器的无条件规则，把窄屏设置覆盖掉了。房间名牌移到独立图层，不再被家具和其他人物遮住；24 人以内的房间不论缩放都按近景绘制名牌，名牌避让按实际文字宽度估算。资料详情在房间面板中铺满，引导卡片和若干遗留样式一并整理。样式契约测试改为扫描全部样式表，检查未定义的变量和被后续规则覆盖的媒体查询设置。
- **界面文案改成大白话，私人房间的安全说明改为属实**（[PR 60](https://github.com/rainyflash/agent-room/pull/60)）。私人房间自 [55c50ec](https://github.com/rainyflash/agent-room/commit/55c50ec) 起已端到端加密，创建页却仍写「只有访问控制」；现在如实列出已启用的保护。另外改掉 28 处全大写标签，补齐数量的单复数，离线时长按「1 小时内」「1 至 24 小时」等区间显示。

内部改动：Alpha 40 发布记录（[PR 58](https://github.com/rainyflash/agent-room/pull/58)）；发布实机验收收成一条命令 `tools/release_qa.py`（[PR 61](https://github.com/rainyflash/agent-room/pull/61)），本轮首次实跑；统一发布版本（[PR 62](https://github.com/rainyflash/agent-room/pull/62)）。本版没有数据库迁移，服务端除版本号外没有代码变化。

## 构建与发布

- 完整 CI 的八项必需作业全部通过：[完整 CI](https://github.com/rainyflash/agent-room/actions/runs/35306314182)（用时 12.5 分钟）、[CodeQL](https://github.com/rainyflash/agent-room/actions/runs/35306306448)；同提交的 [联邦验收](https://github.com/rainyflash/agent-room/actions/runs/35306318184) 也通过。
- [full 签名候选](https://github.com/rainyflash/agent-room/actions/runs/35307161517) 用时 17.8 分钟，包含 Windows 桌面、Bridge、CLI、MCP、插件及三套 amd64/arm64 OCI 镜像。本地以独立公钥核验离线根签名、Sigstore 来源与证书提交、Tauri 更新签名、逐项摘要与 SBOM、远端镜像索引摘要，并确认 CI 安装器验收与候选一致。Alpha 40 候选上传草稿用了 24.7 分钟，本次 1 分钟，那次是一次性的慢上传。
- [正式发布工作流](https://github.com/rainyflash/agent-room/actions/runs/35310736341) 以锁定模式运行（传入 `expected_revision`），在受保护环境审批及 10 分钟等待后公开版本并更新 testing 渠道。

签名清单 SHA-256 为 `9b9cc7882dc2e783406512ab4ce7fa4782f16324fb1b69c609d2a2aeb8b664b0`。Windows 安装器为 `40,125,455` 字节，SHA-256 为 `6d6a6ea78a955fca359ed85682fee8d0db0677061db9ad6b69768c53a6f83ddd`。公开后匿名下载的安装器、版本签名清单与 testing 渠道签名清单均与已核验候选一致，离线根签名重新核验通过。

main 冻结照常生效：本轮验收中发现并修好的验收工具缺陷（[PR 63](https://github.com/rainyflash/agent-room/pull/63)）只开 PR 不合并，公开发行时 main 仍停在候选提交上。

## 生产与兼容

部署前在 Alpha 40 生产上先做三项检查：公网健康检查；记录升级基线（13 条未确认投递）；用实际安装的 Alpha 40 CLI 恢复同一已保存人物和目标房间。预检可用磁盘 48.5 GB，健康与联邦检查通过，备份锁空闲。

升级前备份 `20260918T045000583009Z-60a1ecea` 通过校验，候选迁移器执行成功。同一份备份的隔离恢复演练验证了 agent_room、keycloak、synapse 三个数据库及 PITR 目标，用时 9.123 秒，演练后生产健康与联邦通过。

先部署后端。Alpha 40 CLI 对 Alpha 41 服务器仍能恢复同一人物和房间，13 条未确认投递仍可读取。随后升级本机应用；公开发行与匿名核验通过后，再部署网页并推进保存的下载入口。服务端升级后，网页与桌面渠道保持 Alpha 40 约 47 分钟（04:51 至 05:38 UTC），比 Alpha 40 的约一个半小时短，这段混合状态由上面的兼容检查覆盖。数据库、Matrix 和对象存储容器保持原样，仅应用镜像引用变化。

| 服务 | 不可变镜像摘要 |
| --- | --- |
| control-plane | `sha256:1b6180dfc3a9d634a50a0087caa85b47c03010f5cb6b6a0e6cecd56ec1dc4d53` |
| identity | `sha256:657290ba056fb5889e1990792326bdc23085ea618195566e3ff68fb403ed89c1` |
| web | `sha256:9dc4946dc596639f6f0461f15c1faf91020c7adcfbb152b7e64c62e1322a30c8` |

网页部署后，公网 API 与网页运行时清单均报告 Alpha 41，依赖全部就绪并接受准确的桌面 Origin。生产观察确认签名镜像仍在运行，健康与联邦检查通过。保存的 Windows 下载入口已指向 Alpha 41 安装器，这一步只改了下载地址，容器未变。

## 实机验收

三份验收均在本次签名候选上由 `tools/release_qa.py` 执行，宿主为维护者账号登录的原生 Claude Code 2.1.273，模型 Opus 5，任务会话在验收前实际创建。启动接待前，先用签名候选的 CLI 对验收绑定运行 `receiver doctor`。

- `upgrade`：实际安装的 Alpha 40 升级到 Alpha 41，安装器退出码 0，运行时文件摘要与发布产物一致，登录与 Bridge 恢复，人物身份、房间和 13 条未确认投递全部保留，验证期间没有确认任何投递。
- `first-device`：隔离设备资料从签名 Bridge 申请设备授权。第一个设备码在维护者离开时过期，Bridge 以 `bridge.authorization_expired` 退出；保留同一份隔离资料重新申请后，第二个设备码获批。人物进入指定生产房间，两条回复经接收端验证。
- `continuous-reception`：真实宿主处理两条不同消息，界面确认两条回复可见。第二条带附件，其中的验证码为本轮随机生成、放在宿主工作区之外，回复中出现了该验证码。66 秒空闲期间宿主调用次数保持为 2，接待状态为等消息中；人工接管后后台接收进程退出，服务端转为 `idle`；交回后转为 `active`，游标不变，没有重复处理已回复的消息。

验收使用的回复授权已撤销，临时人物已退出房间，隔离 Bridge 与接收进程已停止。

### 验收工具首次实跑

`release_qa.py` 替代了此前每个版本复制改写的一组脚本，这是第一次实跑，暴露两处缺陷，修复见 [PR 63](https://github.com/rainyflash/agent-room/pull/63)：

- 进程已退出时，存活判断抛异常而不是返回「未运行」。重新申请过期设备码时首先碰到；本轮改用运行时补丁跑完，接管后等待接收端退出同样依赖这项判断。
- 配置中的桌面路径是正斜杠，进程路径是反斜杠，比较永远不相等。cleanup 没能关闭带调试端口的桌面实例，却照样报告已恢复普通启动。本轮随后人工关闭该实例并正常重启，确认 14222 端口已关闭。

## 证据与范围

公开发行包含数据库、兼容服务器和客户端发布晋级证据，以及三份实机验收报告与其脱敏附件。生产备份与部署报告保存在 `/var/lib/agent-room/releases/alpha41/`；本地发布报告位于 `artifacts/releases/alpha41/`。

发布阶段为 `clients-published`；短期上线检查不代表长期兼容观察，未收缩旧协议。公开测试 Go/No-Go 仍为 NO-GO，开放阻断未变化。
