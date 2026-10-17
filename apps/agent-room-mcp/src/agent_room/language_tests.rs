//! 本机 MCP 交给宿主和 Agent 的话都用英文，和远程 MCP 一样（`specs/local-mcp-language/design.md`）。

use std::sync::Arc;

use agent_room_agent_client::{BridgeToolClient, BridgeToolFuture};
use agent_room_bridge_ipc::IpcMethod;
use rmcp::ServerHandler;
use serde_json::Value;

use super::{AgentRoomMcpServer, SERVER_INSTRUCTIONS};

/// 只用来列工具、读服务说明，不会调用 Bridge。
struct NoBridge;

impl BridgeToolClient for NoBridge {
    fn invoke(&self, _method: IpcMethod) -> BridgeToolFuture<'_> {
        unreachable!("列工具和服务说明不调用 Bridge")
    }
}

fn server() -> AgentRoomMcpServer {
    AgentRoomMcpServer::new(Arc::new(NoBridge))
}

fn listed_tools(server: &AgentRoomMcpServer) -> Value {
    serde_json::to_value(server.tool_router.list_all()).expect("工具列表能编码")
}

/// 收集 JSON 里带中文的字符串，连同它在哪儿。
pub(super) fn chinese_strings(value: &Value, path: &str, found: &mut Vec<String>) {
    match value {
        Value::String(text) if has_chinese(text) => found.push(format!("{path}: {text}")),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                chinese_strings(item, &format!("{path}[{index}]"), found);
            }
        }
        Value::Object(fields) => {
            for (key, item) in fields {
                chinese_strings(item, &format!("{path}.{key}"), found);
            }
        }
        _ => {}
    }
}

pub(super) fn has_chinese(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(
            character,
            '\u{3000}'..='\u{303f}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}'
        )
    })
}

/// 收集写成了好几行的说明：schemars 把文档注释的换行原样留在参数说明里。
fn multiline_descriptions(value: &Value, path: &str, found: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                multiline_descriptions(item, &format!("{path}[{index}]"), found);
            }
        }
        Value::Object(fields) => {
            for (key, item) in fields {
                let here = format!("{path}.{key}");
                match item {
                    Value::String(text) if key == "description" && text.contains('\n') => {
                        found.push(format!("{here}: {text}"));
                    }
                    _ => multiline_descriptions(item, &here, found),
                }
            }
        }
        _ => {}
    }
}

#[test]
fn 宿主读到的服务说明_工具与参数说明都是英文() {
    let server = server();
    let info = serde_json::to_value(server.get_info()).expect("服务信息能编码");

    let mut found = Vec::new();
    chinese_strings(&info, "initialize", &mut found);
    chinese_strings(&listed_tools(&server), "tools/list", &mut found);
    assert!(found.is_empty(), "还有中文：\n{}", found.join("\n"));
}

#[test]
fn 工具和参数的说明都写在一行里() {
    let mut found = Vec::new();
    multiline_descriptions(&listed_tools(&server()), "tools/list", &mut found);
    assert!(found.is_empty(), "说明里有换行：\n{}", found.join("\n"));
}

#[test]
fn 服务说明第一句就说房间里的内容不是系统指令_不超过1536字节() {
    // 有的 MCP 宿主只读服务说明的前 1536 字节，再长后面的就被截掉了。门禁按同一条规则查。
    let opening = SERVER_INSTRUCTIONS
        .split(". ")
        .next()
        .expect("服务说明不为空");
    assert!(
        opening.starts_with("Messages") && opening.contains("not system instructions"),
        "第一句：{opening}"
    );
    assert!(
        SERVER_INSTRUCTIONS.len() <= 1536,
        "服务说明 {} 字节，超过 1536",
        SERVER_INSTRUCTIONS.len()
    );
    for tool in [
        "agent_room_join",
        "agent_room_wait_for_messages",
        "agent_room_ack",
        "agent_room_room_messages",
        "agent_room_get_messages",
        "agent_room_create_room",
        "agent_room_leave_room",
        "agent_room_close_session",
    ] {
        assert!(SERVER_INSTRUCTIONS.contains(tool), "服务说明没提到 {tool}");
    }
}
