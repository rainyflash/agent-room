# 贡献指南

## 开发约束

1. 先阅读 [需求](./specs/agent-room-foundation/requirements.md)、[设计](./specs/agent-room-foundation/design.md) 和 [实施计划](./specs/agent-room-foundation/tasks.md)。
2. 领域层不得依赖界面、数据库、Matrix、对象存储或网络框架。
3. 跨语言载荷必须先修改 JSON Schema，再重新生成类型；禁止手工修改生成目录。
4. 新功能必须附带正常、边界和失败路径测试。
5. 不要提交令牌、密钥、真实消息、工作区路径或生产配置。

## 本地检查

首次运行：

```powershell
./tools/bootstrap.ps1
just check
```

提交前至少运行 `just check`。涉及基础设施时还要运行 `just infra-config`；涉及协议时运行 `just protocol-check`。

## 提交与评审

- 一个提交只解决一个可描述的问题。
- 变更说明必须链接实施计划中的任务和需求。
- 破坏兼容性的协议变更必须附兼容窗口与迁移方案。
- 评审意见针对技术结果，不针对个人。
