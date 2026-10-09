//! 围观页的样子：从解析、验过签的消息和在线名单拼出回给网页的 JSON。
//!
//! 只给公开展示得了的：没有 Matrix ID、`principalId`、头像和附件内容；敏感、受限的只说有一条；
//! 被隐藏、撤回的不给。

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use agent_room_application::ports::{SecretFactory, SecretGenerationFailure, SecretValue};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    messages::{MessageProjectionMutation, ProjectedMessageActor, ProjectedMessagePreview},
    presence::PresenceObservation,
};
use agent_room_domain::{
    agent_status::AgentWorkStatus,
    ids::{AgentId, MessageId},
    messages::{MessagePreview, MessageRelation, MessageRevisionKind, MessageSensitivity},
    time::UtcMillis,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;

use super::WatchLobby;

const SCHEMA_VERSION: u8 = 1;
/// 最多给最近这么多条消息。
pub(super) const MAX_MESSAGES: usize = 50;
/// 正文最多给这么多个字符，再长截断。
pub(super) const MAX_TEXT_CHARACTERS: usize = 1_000;
/// 在线的 Agent 最多列这么多个。
pub(super) const MAX_ONLINE_AGENTS: usize = 100;
/// 编号取摘要的前这么多字节：72 位，一个快照里不会撞。
const KEY_DIGEST_BYTES: usize = 9;

/// 读一个大厅分片得到的、验过的东西。
pub(super) struct RoomReading {
    /// 消息与修订，按时间先后。
    pub(super) mutations: Vec<MessageProjectionMutation>,
    /// 此刻在线（含重连宽限）的 Agent。
    pub(super) online: Vec<PresenceObservation>,
    /// 管理员隐藏的消息的事件 ID。
    pub(super) hidden: HashSet<String>,
    /// 其中是网络 Agent 的。
    pub(super) network_agents: HashSet<AgentId>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WatchView {
    schema_version: u8,
    lobby: LobbyView,
    participants: Vec<ParticipantView>,
    messages: Vec<MessageView>,
    updated_at_unix_ms: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LobbyView {
    /// 公开大厅目录里的编号（`/lobbies/public` 不登录也给），网页拿它链到“进去说话”。
    catalog_id: String,
    name: String,
    slug: String,
}

#[derive(Debug, Serialize)]
struct ParticipantView {
    key: String,
    name: String,
    kind: ParticipantKind,
    /// 只有 Agent 有在线状态；人一律是 false。
    online: bool,
    /// 在线的 Agent 自己报的粗略状态（`idle`、`working` 这些，大厅里谁都看得到）；不在线的和人是 null。
    status: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum ParticipantKind {
    Person,
    Agent,
    NetworkAgent,
}

// 回给网页的线上格式：几个互不相干的标记平铺着最好读。
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MessageView {
    key: String,
    /// 说话的人在 `participants` 里的编号。
    author: String,
    /// 正文，最多 1000 个字符；不公开的消息是空的。
    text: String,
    truncated: bool,
    /// 敏感或受限的消息：网页只说“一条不公开的消息”。
    withheld: bool,
    /// 带了文件（只说有，不给内容和文件名）。
    attachment: bool,
    edited: bool,
    /// 回复的那条的编号；那条不在这一页里时网页说“回复一条更早的消息”。
    reply_to: Option<String>,
    sent_at_unix_ms: i64,
}

/// 快照里的人和消息用的编号：加了盐的摘要，不是 Matrix 的用户 ID、事件 ID。盐每次启动随机生成，
/// 同一进程里同一个人、同一条消息的编号不变，网页才能平滑地追加新消息。
pub(super) struct Keys {
    secrets: Arc<dyn SecretFactory>,
    salt: SecretValue,
}

impl Keys {
    pub(super) fn new(secrets: Arc<dyn SecretFactory>) -> Result<Self, SecretGenerationFailure> {
        let salt = secrets.generate()?;
        Ok(Self { secrets, salt })
    }

    fn participant(&self, matrix_user_id: &str) -> String {
        self.key("p", matrix_user_id)
    }

    fn message(&self, message_id: MessageId) -> String {
        self.key("m", &message_id.to_string())
    }

    fn key(&self, kind: &str, value: &str) -> String {
        let digest = self
            .secrets
            .digest(&format!("{}\u{1f}{kind}\u{1f}{value}", self.salt.expose()));
        format!(
            "{kind}{}",
            URL_SAFE_NO_PAD.encode(&digest.as_bytes()[..KEY_DIGEST_BYTES])
        )
    }
}

/// 还没人进过的大厅（`reading` 是 `None`）人和消息都是空的。
pub(super) fn assemble(
    lobby: &WatchLobby,
    reading: Option<RoomReading>,
    keys: &Keys,
    now: UtcMillis,
) -> WatchView {
    let (participants, messages) = reading
        .map(|reading| content(&reading, keys))
        .unwrap_or_default();
    WatchView {
        schema_version: SCHEMA_VERSION,
        lobby: LobbyView {
            catalog_id: lobby.catalog_id.to_string(),
            name: lobby.name.clone(),
            slug: lobby.slug.clone(),
        },
        participants,
        messages,
        updated_at_unix_ms: now.value(),
    }
}

/// 消息作者和在线名单里出现的 Agent：要问其中哪些是网络 Agent。
pub(super) fn agent_ids(
    mutations: &[MessageProjectionMutation],
    online: &[PresenceObservation],
) -> Vec<AgentId> {
    let mut ids: Vec<AgentId> = mutations
        .iter()
        .filter_map(|mutation| match mutation {
            MessageProjectionMutation::Preview(preview) => preview.actor.agent_identity(),
            MessageProjectionMutation::Revision(_) => None,
        })
        .map(BridgeAgentIdentity::agent_id)
        .chain(
            online
                .iter()
                .map(|entry| entry.presence().identity().agent_id()),
        )
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn content(reading: &RoomReading, keys: &Keys) -> (Vec<ParticipantView>, Vec<MessageView>) {
    let shown = visible_messages(&reading.mutations, &reading.hidden);
    let mut participants = Participants {
        keys,
        network_agents: &reading.network_agents,
        list: Vec::new(),
        seen: HashSet::new(),
    };
    // 先列在线的 Agent（按名字），再列最近说过话的（新的在前）。
    let mut online: Vec<&PresenceObservation> = reading.online.iter().collect();
    online.sort_by_cached_key(|entry| entry.presence().identity().display_name().to_lowercase());
    for entry in online.into_iter().take(MAX_ONLINE_AGENTS) {
        participants.add_online(entry);
    }
    for message in shown.iter().rev() {
        participants.add_author(&message.original.actor);
    }
    let messages = shown
        .iter()
        .map(|message| message_view(message, keys))
        .collect();
    (participants.list, messages)
}

/// 一条要显示的消息：原来的那条（作者、编号、时间、回复对象不变），和改过以后的预览。
struct Shown<'a> {
    original: &'a ProjectedMessagePreview,
    current: &'a MessagePreview,
    edited: bool,
}

/// 按时间先后应用修订：作者自己改的换成新预览，撤回的和管理员撤下的拿掉；再拿掉被隐藏的，
/// 留最近 50 条。和网页一样，只认作者自己发的修订。
fn visible_messages<'a>(
    mutations: &'a [MessageProjectionMutation],
    hidden: &HashSet<String>,
) -> Vec<Shown<'a>> {
    let mut order: Vec<MessageId> = Vec::new();
    let mut messages: HashMap<MessageId, Shown<'a>> = HashMap::new();
    let mut removed: HashSet<MessageId> = HashSet::new();
    for mutation in mutations {
        match mutation {
            MessageProjectionMutation::Preview(preview) => {
                if messages.contains_key(&preview.message_id) {
                    continue;
                }
                order.push(preview.message_id);
                messages.insert(
                    preview.message_id,
                    Shown {
                        original: preview,
                        current: &preview.preview,
                        edited: false,
                    },
                );
            }
            MessageProjectionMutation::Revision(revision) => {
                let Some(target) = messages.get_mut(&revision.target_message_id) else {
                    continue;
                };
                if author_id(&target.original.actor) != author_id(&revision.actor) {
                    continue;
                }
                match revision.kind {
                    MessageRevisionKind::Replace => {
                        if let Some(preview) = &revision.preview {
                            target.current = preview;
                            target.edited = true;
                        }
                    }
                    MessageRevisionKind::Redact | MessageRevisionKind::Moderate => {
                        removed.insert(revision.target_message_id);
                    }
                }
            }
        }
    }
    let mut shown: Vec<Shown<'a>> = order
        .into_iter()
        .filter(|id| !removed.contains(id))
        .filter_map(|id| messages.remove(&id))
        .filter(|message| !hidden.contains(message.original.event_id.as_str()))
        .collect();
    let overflow = shown.len().saturating_sub(MAX_MESSAGES);
    shown.drain(..overflow);
    shown
}

fn message_view(message: &Shown<'_>, keys: &Keys) -> MessageView {
    let withheld = message.current.sensitivity() != MessageSensitivity::Normal;
    let (text, attachment) = if withheld {
        (String::new(), false)
    } else {
        text_of(message.current)
    };
    let (text, truncated) = truncate(text);
    MessageView {
        key: keys.message(message.original.message_id),
        author: keys.participant(author_id(&message.original.actor)),
        text,
        truncated,
        withheld,
        attachment,
        edited: message.edited,
        reply_to: message.original.relation.map(|relation| match relation {
            MessageRelation::ReplyTo(target) => keys.message(target),
        }),
        sent_at_unix_ms: message
            .original
            .origin_server_timestamp
            .and_then(|timestamp| i64::try_from(timestamp).ok())
            .unwrap_or_else(|| message.original.created_at.value()),
    }
}

/// 聊天消息给正文，带文件的只说带了；不是聊天的（比如交接的文件）给摘要，内容当文件。
fn text_of(preview: &MessagePreview) -> (String, bool) {
    if let Some(chat) = preview.conversation() {
        return (chat.text().to_owned(), chat.attachment_name().is_some());
    }
    let summary = preview.summary().as_str();
    let text = if summary.trim().is_empty() {
        preview.title().as_str()
    } else {
        summary
    };
    (text.to_owned(), true)
}

fn truncate(text: String) -> (String, bool) {
    match text.char_indices().nth(MAX_TEXT_CHARACTERS) {
        Some((cut, _)) => (text[..cut].to_owned(), true),
        None => (text, false),
    }
}

fn author_id(actor: &ProjectedMessageActor) -> &str {
    match actor {
        ProjectedMessageActor::Agent { identity, .. } => identity.matrix_user_id().as_str(),
        ProjectedMessageActor::Human { matrix_user_id, .. } => matrix_user_id.as_str(),
    }
}

/// 名单：每个人只列一次，先到的为准。
struct Participants<'a> {
    keys: &'a Keys,
    network_agents: &'a HashSet<AgentId>,
    list: Vec<ParticipantView>,
    seen: HashSet<String>,
}

impl Participants<'_> {
    fn add_online(&mut self, entry: &PresenceObservation) {
        let identity = entry.presence().identity();
        let kind = self.agent_kind(identity.agent_id());
        self.add(
            identity.matrix_user_id().as_str(),
            identity.display_name(),
            kind,
            Some(entry.status()),
        );
    }

    fn add_author(&mut self, actor: &ProjectedMessageActor) {
        match actor {
            ProjectedMessageActor::Agent { identity, .. } => {
                let kind = self.agent_kind(identity.agent_id());
                self.add(
                    identity.matrix_user_id().as_str(),
                    identity.display_name(),
                    kind,
                    None,
                );
            }
            ProjectedMessageActor::Human {
                display_name,
                matrix_user_id,
                ..
            } => self.add(
                matrix_user_id.as_str(),
                display_name,
                ParticipantKind::Person,
                None,
            ),
        }
    }

    /// `status` 只有在线的 Agent 才有。
    fn add(
        &mut self,
        matrix_user_id: &str,
        name: &str,
        kind: ParticipantKind,
        status: Option<AgentWorkStatus>,
    ) {
        let key = self.keys.participant(matrix_user_id);
        if !self.seen.insert(key.clone()) {
            return;
        }
        self.list.push(ParticipantView {
            key,
            name: name.to_owned(),
            kind,
            online: status.is_some(),
            status: status.map(AgentWorkStatus::as_str),
        });
    }

    fn agent_kind(&self, agent_id: AgentId) -> ParticipantKind {
        if self.network_agents.contains(&agent_id) {
            ParticipantKind::NetworkAgent
        } else {
            ParticipantKind::Agent
        }
    }
}
