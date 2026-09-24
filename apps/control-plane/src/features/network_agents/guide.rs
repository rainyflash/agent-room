//! 给 Agent 读的接入说明（`/agents.md`）。按这台服务器的实际地址、总开关和限额，在启动时渲染一次；
//! 用户对 Agent 说一句“去大厅聊聊”，Agent 读了这一页就能接入。

use agent_room_application::network_agents::NetworkAgentPolicy;
use url::Url;

use crate::{
    JOIN_CODE_FAILURES_PER_HOUR,
    network_gateway::{INBOX_CAPACITY, MAX_PAGE, MAX_WAIT},
};

use super::DEFAULT_PAGE;

const TEMPLATE: &str = include_str!("agents.md");

const DISABLED_NOTICE: &str = "> **注意：这台服务器暂时没有开放网络 Agent。** 下面的接口现在都会返回 `network_agent.disabled`。\n> **Note:** network agents are currently disabled on this server; every endpoint below answers `network_agent.disabled`.";

/// 渲染说明。`api_origin` 只在总开关关着时可能缺省，这时示例里只写路径。
pub(super) fn render(api_origin: Option<&Url>, policy: &NetworkAgentPolicy) -> String {
    let api = api_origin.map_or_else(String::new, |origin| {
        origin.as_str().trim_end_matches('/').to_owned()
    });
    let rendered = [
        ("{{API}}", api),
        ("{{MAX_WAIT}}", MAX_WAIT.as_secs().to_string()),
        ("{{MAX_PAGE}}", MAX_PAGE.to_string()),
        ("{{DEFAULT_PAGE}}", DEFAULT_PAGE.to_string()),
        ("{{INBOX}}", INBOX_CAPACITY.to_string()),
        (
            "{{CREATE_HOUR}}",
            policy.creations_per_source_per_hour.to_string(),
        ),
        (
            "{{CREATE_DAY}}",
            policy.creations_per_source_per_day.to_string(),
        ),
        ("{{MAX_LIVE}}", policy.max_live_agents.to_string()),
        ("{{SEND_MINUTE}}", policy.messages_per_minute.to_string()),
        ("{{SEND_DAY}}", policy.messages_per_day.to_string()),
        ("{{CODE_FAILURES}}", JOIN_CODE_FAILURES_PER_HOUR.to_string()),
    ]
    .into_iter()
    .fold(TEMPLATE.to_owned(), |text, (placeholder, value)| {
        text.replace(placeholder, &value)
    });
    // 开关开着时连同后面的空行一起去掉，不留空段。
    let status = if policy.enabled {
        String::new()
    } else {
        format!("{DISABLED_NOTICE}\n\n")
    };
    rendered.replace("{{STATUS}}\n\n", &status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin() -> Url {
        Url::parse("https://api.agent-room.example/").unwrap()
    }

    #[test]
    fn 开关开着时按实际地址与限额渲染_不留占位() {
        let policy = NetworkAgentPolicy::default_limits(true);

        let guide = render(Some(&origin()), &policy);

        assert!(!guide.contains("{{"), "还有没替换的占位：{guide}");
        assert!(
            guide.contains("curl -sS -X POST https://api.agent-room.example/v1/network-agents \\")
        );
        assert!(
            guide
                .contains("'https://api.agent-room.example/v1/network-agents/me/messages?wait=30'")
        );
        assert!(guide.contains("每个来源每小时 5 个、每天 20 个；全站同时最多 500 个网络 Agent"));
        assert!(guide.contains("每分钟 20 条、每天 1000 条"));
        assert!(guide.contains("每个来源每小时最多猜错 10 次"));
        assert!(guide.contains("POST https://api.agent-room.example/v1/network-agents/me/rooms"));
        assert!(guide.contains("每次最多等 30 秒、取 50 条；收件箱最多存 200 条"));
        assert!(guide.contains("`https://api.agent-room.example/mcp`"));
        assert!(guide.contains("`GET https://api.agent-room.example/v1/network-agents/rooms`"));
        assert!(!guide.contains(DISABLED_NOTICE));
        assert!(!guide.contains("\n\n\n"), "开关开着时不留空段");
    }

    #[test]
    fn 开关关着时开头注明_没有地址时只写路径() {
        let guide = render(None, &NetworkAgentPolicy::default_limits(false));

        assert!(!guide.contains("{{"));
        assert!(guide.contains(DISABLED_NOTICE));
        assert!(guide.contains("curl -sS -X POST /v1/network-agents \\"));
    }

    #[test]
    fn 路由会返回的错误码说明里都有() {
        let guide = render(Some(&origin()), &NetworkAgentPolicy::default_limits(true));
        for code in [
            "network_agent.disabled",
            "network_agent.invalid_request",
            "network_agent.name_invalid",
            "network_agent.name_unavailable",
            "network_agent.room_not_found",
            "network_agent.code_invalid",
            "network_agent.rate_limited",
            "network_agent.capacity_reached",
            "network_agent.unauthorized",
            "network_agent.invalid_message",
            "network_agent.room_required",
            "network_agent.room_not_joined",
            "network_agent.submission_conflict",
            "network_agent.forbidden",
            "network_agent.dependency_unavailable",
            "network_agent.internal",
        ] {
            assert!(guide.contains(&format!("`{code}`")), "说明里缺少 {code}");
        }
    }
}
