# ADR 0007：拆分客户端与服务端发布边界

- 状态：Accepted
- 日期：2026-08-29

## 背景

早期 Alpha 流水线把 Windows 客户端、三个服务端镜像、两种 Linux 架构、逐件 SBOM、Sigstore 证明和最终发布绑定为一个候选。任何桌面端小改动都会启动完整服务端构建。2026-08-29 的 GitHub Billing 快照显示：Agent Room 当月 Actions 毛用量为 `$96.28`、实际应付为 `$0`；其中 macOS 3-core 为 `$43.93`、Windows 为 `$28.83`、Linux 为 `$22.76`。即使公开仓库折扣覆盖了账单，累计 Runner 时间仍显著拖慢反馈，并制造接近缓存上限的 BuildKit 数据。

## 决策

发布候选显式选择以下 profile：

- `client`：默认路径，只构建 Windows 桌面端、Bridge、通用 MCP、宿主适配器、更新清单及其签名证据；
- `full`：在 `client` 产物之外，构建 `control-plane`、`identity`、`web` 的 amd64/arm64 OCI Index 及其签名证据。

两个 profile 共用同一 testing 根清单、Tauri 更新签名和最终密码学验证。客户端候选在公开前仍必须提供 `compatible-server` 晋级证据，因此拆分构建不等于绕过协议兼容门禁。

每次 push 只运行格式、类型、单元测试、浏览器验收和必要的 Windows 原生检查。真实 PostgreSQL/Matrix 集成、依赖审计与完整 SBOM 改为显式手动深度验证。macOS 继续只允许维护者自托管 Runner 手动执行。

Bridge、MCP 与桌面壳共享同一个 Windows Runtime 作业和缓存边界，避免为同一依赖图重复启动 Runner、检出仓库和安装工具链。Actions 临时 Artifact 不再被当成长期发布存储；GitHub Draft/Public Release 才是发布资产的唯一长期来源。Docker 缓存使用最小模式，Rust Release 构建使用受限的依赖缓存。

## 后果

- 日常 Alpha 不再启动九个服务端镜像相关作业；
- 普通 CI 的两个 Windows 作业合并为一个，平台编译结果可以在同一工作区复用；
- 客户端与服务端可以按真实变更频率独立构建；
- 维护者在服务端需要交付时必须明确选择 `full`；
- 深度验证不再自动消耗每次提交的反馈时间，正式发布或高风险变更前必须主动运行；
- stable 发布仍需完整候选、离线根和独立审批，本 ADR 不降低 ADR 0005 的目标。

## 重新评估条件

当项目拥有稳定的多维护者发布团队，或服务端与客户端形成独立版本号时，重新评估是否把两个 profile 拆成完全独立的版本序列与仓库级发布流水线。
