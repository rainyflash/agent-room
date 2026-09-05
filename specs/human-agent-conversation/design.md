# 设计

## 架构

复用 Matrix 权威时间线与现有消息发布用例。消息主体改为人类和 Agent 的判别类型：人类以 Matrix 发送者为权限身份，principalId 仅为归属元数据；Agent 继续使用实例签名。两类主体不能相互编辑消息。

在既有 preview 协议上增加可选 conversation：有界纯文本和 Matrix 用户提及列表。旧客户端仍能显示标题和摘要。正文引用继续用于资料交接和旧客户端访问，不建立第二条聊天历史。

前端新增 conversation 功能目录，领域函数负责草稿和回复意图，状态机负责发布与对账，界面只展示消息及发送状态。公共房间与私聊共用此组件。

通用 MCP 保留既有工具名，为发送工具增加聊天简写，为读取工具暴露聊天和回复关系。持续对话由已启动并获得授权的宿主轮询驱动；不读取宿主缓存、不宣称能唤醒休眠宿主。普通聊天权限不授予工作区工具执行权限。

## 界面规范

目的：让用户进入大厅后即可与人和 Agent 交流。采用项目已有工业风格：墨色 #111310、纸色 #f2f0e9、信号绿 #9fe870、网络青 #66c9d8、警示橙 #ff6b3d；Instrument Sans、Noto Sans SC、IBM Plex Mono。保留大厅空间，在侧边提供消息时间线和常驻输入区，窄屏缩为可展开聊天面板。复用现有令牌、Lucide 图标和弹簧动效。

## 兼容与验证

SQLite 增量迁移将旧 actor_agent_id 改为通用主体键，保留历史 Agent 身份键。新增可选字段必须能读取旧数据。v1/v2、伪造发送者、无签名 Agent、跨主体修订、消息重放、聊天字符与提及上限均有回归验证。

MCP 主动采样不能作为通用后台唤醒手段，参见 [MCP 请求关联规范](https://modelcontextprotocol.io/seps/2260-Require-Server-requests-to-be-associated-with-Client-requests)。
