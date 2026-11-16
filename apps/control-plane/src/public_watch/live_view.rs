//! 分享页实况的样子（`/r/<码>/live`）：消息那一部分和围观公开大厅一样，只给开实况以后说的最近 20 句、
//! 每句开头 280 个字；不列在场的人和 Agent，名单里只有说过话的，加上在线数。

use std::collections::{HashMap, HashSet};

use agent_room_application::{
    public_room_directory::PublicRoomOnline, room_people::shown_display_name,
};
use agent_room_bridge_core::messages::{MessageProjectionMutation, ProjectedMessageActor};
use agent_room_domain::{ids::AgentId, time::UtcMillis};
use serde::Serialize;

use super::view::{self, Keys, MessageView, ParticipantKind};

const SCHEMA_VERSION: u8 = 1;
/// 最多给最近这么多句。
pub(super) const LIVE_MESSAGES: usize = 20;
/// 每句最多给这么多个字符。
pub(super) const LIVE_TEXT_CHARACTERS: usize = 280;

/// 拼一份实况要用到的、这次读到和记着的东西。
pub(super) struct LiveReading<'a> {
    /// 消息与修订，按时间先后。
    pub(super) mutations: &'a [MessageProjectionMutation],
    /// 管理员隐藏的消息的事件 ID。
    pub(super) hidden: &'a HashSet<String>,
    pub(super) network_agents: &'a HashSet<AgentId>,
    /// 本机 Agent 的主人（给别人看的名字）；网络 Agent、不在房间里的没有。
    pub(super) owners: &'a HashMap<AgentId, Option<String>>,
    pub(super) online: Option<PublicRoomOnline>,
    /// 开实况的那一刻：早于它的不给。
    pub(super) enabled_at: UtcMillis,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LiveView {
    schema_version: u8,
    /// 现在几个在线，和分享页卡片上的是同一个数；问不到时为空。
    online: Option<OnlineView>,
    participants: Vec<LiveParticipantView>,
    messages: Vec<MessageView>,
    updated_at_unix_ms: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OnlineView {
    count: u32,
    at_least: bool,
}

#[derive(Debug, Serialize)]
struct LiveParticipantView {
    key: String,
    name: String,
    kind: ParticipantKind,
    /// 本机 Agent 是谁的（主人的名字，有 @ 的只留前面那段）；人、网络 Agent 和问不到的为空。
    owner: Option<String>,
}

pub(super) fn assemble(reading: &LiveReading<'_>, keys: &Keys, now: UtcMillis) -> LiveView {
    let mut shown: Vec<_> = view::visible_messages(reading.mutations, reading.hidden)
        .into_iter()
        .filter(|message| view::sent_at(message) >= reading.enabled_at.value())
        .collect();
    let overflow = shown.len().saturating_sub(LIVE_MESSAGES);
    shown.drain(..overflow);
    let mut participants = Vec::new();
    let mut seen = HashSet::new();
    // 名单里新说话的在前，和围观公开大厅一样。
    for message in shown.iter().rev() {
        let participant = participant(&message.original.actor, reading, keys);
        if seen.insert(participant.key.clone()) {
            participants.push(participant);
        }
    }
    LiveView {
        schema_version: SCHEMA_VERSION,
        online: reading.online.map(|online| OnlineView {
            count: online.count,
            at_least: online.at_least,
        }),
        participants,
        messages: shown
            .iter()
            .map(|message| view::message_view(message, keys, LIVE_TEXT_CHARACTERS))
            .collect(),
        updated_at_unix_ms: now.value(),
    }
}

/// 说话的 Agent：本机的、网络的。
pub(super) fn agent_authors(mutations: &[MessageProjectionMutation]) -> Vec<AgentId> {
    view::agent_ids(mutations, &[])
}

fn participant(
    actor: &ProjectedMessageActor,
    reading: &LiveReading<'_>,
    keys: &Keys,
) -> LiveParticipantView {
    let key = keys.participant(view::author_id(actor));
    match actor {
        ProjectedMessageActor::Agent { identity, .. } => {
            let agent_id = identity.agent_id();
            let network = reading.network_agents.contains(&agent_id);
            LiveParticipantView {
                key,
                name: identity.display_name().to_owned(),
                kind: if network {
                    ParticipantKind::NetworkAgent
                } else {
                    ParticipantKind::Agent
                },
                owner: if network {
                    None
                } else {
                    reading.owners.get(&agent_id).cloned().flatten()
                },
            }
        }
        ProjectedMessageActor::Human { display_name, .. } => LiveParticipantView {
            key,
            name: shown_display_name(display_name),
            kind: ParticipantKind::Person,
            owner: None,
        },
    }
}
