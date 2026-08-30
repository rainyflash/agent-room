use crate::generated::{Actor, ActorRef, AgentActor, AgentProvenance, Provenance};

/// 把 v1 的无判别 Agent 主体投影成 v2 的显式 Agent 主体。
///
/// v1 的 `human` 没有 Principal 标识，不能安全伪造 `HumanActor`，因此保守映射为
/// `human_confirmed_agent`。
#[must_use]
pub fn project_legacy_actor(actor: ActorRef) -> Actor {
    let provenance = match actor.provenance {
        Provenance::AutonomousAgent => AgentProvenance::AutonomousAgent,
        Provenance::Human | Provenance::HumanConfirmedAgent => AgentProvenance::HumanConfirmedAgent,
    };

    Actor::Agent(AgentActor {
        agent: actor.agent,
        instance_id: actor.instance_id,
        kind: "agent".to_owned(),
        provenance,
        extensions: actor.extensions,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::generated::{ActorRef, AgentProvenance, AgentRef, Provenance};

    use super::{Actor, project_legacy_actor};

    #[test]
    fn 旧人类来源不会被伪造成缺少主体标识的人类() {
        let projected = project_legacy_actor(ActorRef {
            agent: AgentRef {
                agent_id: "01945c1e-7b5a-7c7f-8a28-2de53f56a9a3".to_owned(),
                avatar_url: None,
                display_name: "构建助手".to_owned(),
                matrix_user_id: "@build-agent:agent-room.test".to_owned(),
                extensions: BTreeMap::new(),
            },
            instance_id: "01945c1e-7b5a-7c7f-8a28-2de53f56a9a4".to_owned(),
            provenance: Provenance::Human,
            extensions: BTreeMap::new(),
        });

        let Actor::Agent(actor) = projected else {
            panic!("v1 Agent 主体只能投影成 AgentActor");
        };
        assert_eq!(actor.kind, "agent");
        assert_eq!(actor.provenance, AgentProvenance::HumanConfirmedAgent);
    }
}
