//! 哪些 Agent 是网络 Agent：围观公开大厅和分享页的实况都要分开写。答案不会变，问过的记着。

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use agent_room_application::ports::NetworkAgentLookup;
use agent_room_domain::ids::AgentId;

/// 最多记这么多个，满了清空重来。
const MAX_KNOWN_AGENT_KINDS: usize = 10_000;
/// 一次最多问这么多个。
const LOOKUP_BATCH: usize = 100;

pub(super) struct AgentKinds {
    lookup: Arc<dyn NetworkAgentLookup>,
    kinds: Mutex<HashMap<AgentId, bool>>,
}

impl AgentKinds {
    pub(super) fn new(lookup: Arc<dyn NetworkAgentLookup>) -> Self {
        Self {
            lookup,
            kinds: Mutex::new(HashMap::new()),
        }
    }

    /// 其中哪些是网络 Agent。查不了的这次先当普通 Agent，下次再问。
    pub(super) async fn network_agent_ids(&self, candidates: &[AgentId]) -> HashSet<AgentId> {
        let unknown: Vec<AgentId> = {
            let kinds = self.kinds();
            candidates
                .iter()
                .filter(|id| !kinds.contains_key(id))
                .copied()
                .collect()
        };
        for batch in unknown.chunks(LOOKUP_BATCH) {
            match self.lookup.network_agent_ids(batch).await {
                Ok(found) => {
                    let found: HashSet<AgentId> = found.into_iter().collect();
                    let mut kinds = self.kinds();
                    if kinds.len() + batch.len() > MAX_KNOWN_AGENT_KINDS {
                        kinds.clear();
                    }
                    for id in batch {
                        kinds.insert(*id, found.contains(id));
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        operation = error.operation(),
                        failure = ?error.kind(),
                        "不登录看的页面：查不了哪些是网络 Agent"
                    );
                    break;
                }
            }
        }
        let kinds = self.kinds();
        candidates
            .iter()
            .filter(|id| kinds.get(id) == Some(&true))
            .copied()
            .collect()
    }

    fn kinds(&self) -> MutexGuard<'_, HashMap<AgentId, bool>> {
        self.kinds.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
