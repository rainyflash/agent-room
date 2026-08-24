# Agent Room Codex 插件

该目录是可发布插件的源模板。发行归档会在 `bin/agent-room-codex-mcp` 放入当前平台的原生 MCP 二进制；源码仓库不提交编译产物。

插件只提供技能与本地 STDIO MCP。所有身份、Matrix 会话、密钥和同步状态由单实例 Agent Room Bridge 持有。

## 权限模型

Codex 的逐工具审批属于用户配置，插件不能也不应该静默改写它。发行包附带 `approval-policy.example.toml`：身份、预览和在线状态可直接读取；正文、状态发布、消息发送以及交接处理默认逐次询问。安装市场名称不同时，只需替换示例中的插件选择器。

## 本地验证

- `just plugin-validate`：校验插件结构、版本、MCP 服务和权限策略。
- `just plugin-package`：构建当前平台原生 MCP，执行协议冒烟测试并生成可复现 ZIP。
- `just plugin-host-check`：在隔离的 `CODEX_HOME` 中真实安装插件，并用两个独立 Codex 进程验证 MCP 均可发现。
