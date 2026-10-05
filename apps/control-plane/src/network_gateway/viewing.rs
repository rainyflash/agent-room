//! 按需查看（`specs/agent-reading/design.md` 第 5 步）：按 ID 取、看前后、往前（往后）翻。
//!
//! 都从网络 Agent 的消息记录里读（每个房间留最近几百条），不动收件箱，只给它所在房间里的。
//! 按 ID 取给全文；看前后、往前翻和本机一样，长消息只给开头，要全文再按 ID 取。

use std::collections::HashSet;

use agent_room_application::{
    network_agents::NetworkAgentSession,
    ports::{
        MatrixEventId, MatrixRoomId, NetworkAgentHistoryDirection, NetworkAgentHistoryFilter,
        NetworkAgentHistorySender, NetworkAgentMessageRef, NetworkAgentStoredMessage,
    },
};
use agent_room_bridge_ipc::{
    limits::{AROUND_MESSAGES, MESSAGE_FROM_BYTES, MESSAGE_LOOKUP_IDS, PREVIEW_PAGE_SIZE},
    previews::truncate_preview_value,
};
use agent_room_domain::ids::MessageId;
use serde_json::Value;
use uuid::{Uuid, Version};

use super::{NetworkGateway, NetworkGatewayFailure, session_room};

/// 按 ID 取到的消息：给全文，按要的顺序。`missing` 是找不到、或不在它所在房间里的。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NetworkAgentFoundMessages {
    pub(crate) messages: Vec<Value>,
    pub(crate) missing: Vec<String>,
}

/// 一个房间里的一段消息。`next_cursor` 是接着翻的位置，没有就是翻到头了。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NetworkAgentRoomMessages {
    pub(crate) messages: Vec<Value>,
    pub(crate) next_cursor: Option<String>,
}

/// 看一个房间里之前的消息。`room` 不给时，只在一个房间里就是那间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NetworkAgentRoomMessagesRequest {
    pub(crate) room: Option<String>,
    pub(crate) query: NetworkAgentRoomQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NetworkAgentRoomQuery {
    /// 某一条（事件 ID 或消息 ID）和它前后各几条，早的在前。
    Around { id: String, before: u16, after: u16 },
    /// 往前翻（新的在前）；给了 `after` 就往后翻（旧的在前）。
    History {
        before: Option<String>,
        after: Option<String>,
        limit: u16,
        from: Option<String>,
        mentions_me: bool,
    },
}

impl NetworkAgentRoomQuery {
    /// HTTP 接口和远程 MCP 收到的参数换成一种看法，和本机 MCP 的 `agent_room_room_messages` 一样：
    /// 给了 `around` 就看前后，`limit` 条前后各一半（前面多给一条，每边最多 20 条），另加它本身；
    /// 否则往前（往后）翻。不对时返回是哪一项。
    pub(crate) fn parse(
        around: Option<String>,
        before: Option<String>,
        after: Option<String>,
        limit: u16,
        from: Option<String>,
        mentions_me: bool,
    ) -> Result<Self, &'static str> {
        if !(1..=PREVIEW_PAGE_SIZE).contains(&limit) {
            return Err("limit");
        }
        let Some(id) = around else {
            if before.is_some() && after.is_some() {
                return Err("after");
            }
            return Ok(Self::History {
                before,
                after,
                limit,
                from,
                mentions_me,
            });
        };
        if before.is_some() || after.is_some() || from.is_some() || mentions_me {
            return Err("around");
        }
        // 前面多给一条：被点名时，前面说了什么通常更要紧。
        let after = limit / 2;
        Ok(Self::Around {
            id,
            before: (limit - after).min(AROUND_MESSAGES),
            after: after.min(AROUND_MESSAGES),
        })
    }
}

impl NetworkGateway {
    /// 按 ID 取：事件 ID 或消息 ID，跨房间也行，给全文，按要的顺序；同一条只给一次。
    pub(super) async fn get_messages_internal(
        &self,
        token: &str,
        ids: Vec<String>,
    ) -> Result<NetworkAgentFoundMessages, NetworkGatewayFailure> {
        if ids.is_empty() || ids.len() > MESSAGE_LOOKUP_IDS {
            return Err(NetworkGatewayFailure::InvalidLookup("ids"));
        }
        let refs = ids
            .iter()
            .map(|id| message_ref(id).ok_or(NetworkGatewayFailure::InvalidLookup("ids")))
            .collect::<Result<Vec<_>, _>>()?;
        let session = self
            .agents
            .session(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let found = self
            .history
            .messages_by_id(session.network_agent_id, &refs)
            .await
            .map_err(|_| NetworkGatewayFailure::Unavailable)?;
        let visible: Vec<NetworkAgentStoredMessage> = found
            .into_iter()
            .filter(|message| in_session(&session, &message.room_id))
            .collect();
        let mut given: HashSet<MessageId> = HashSet::new();
        let mut messages = Vec::new();
        let mut missing = Vec::new();
        for (raw, reference) in ids.iter().zip(&refs) {
            match visible.iter().find(|message| matches(message, reference)) {
                Some(message) => {
                    if given.insert(message.message_id) {
                        messages.push(message.preview.clone());
                    }
                }
                None => missing.push(raw.clone()),
            }
        }
        Ok(NetworkAgentFoundMessages { messages, missing })
    }

    /// 看前后，或者往前（往后）翻；只看它所在的房间。
    pub(super) async fn room_messages_internal(
        &self,
        token: &str,
        request: NetworkAgentRoomMessagesRequest,
    ) -> Result<NetworkAgentRoomMessages, NetworkGatewayFailure> {
        let filter = match &request.query {
            NetworkAgentRoomQuery::History {
                from, mentions_me, ..
            } => NetworkAgentHistoryFilter {
                from: from.as_deref().map(sender).transpose()?,
                mentions_me: *mentions_me,
            },
            NetworkAgentRoomQuery::Around { .. } => NetworkAgentHistoryFilter::default(),
        };
        let session = self
            .agents
            .session(token)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let room = session_room(&session, request.room.as_deref())?;
        match request.query {
            NetworkAgentRoomQuery::Around { id, before, after } => {
                self.around(&session, &room, &id, before, after).await
            }
            NetworkAgentRoomQuery::History {
                before,
                after,
                limit,
                ..
            } => {
                let direction = match (before, after) {
                    (Some(before), _) => NetworkAgentHistoryDirection::Before(Some(
                        self.anchor(&session, &room, &before).await?.sequence,
                    )),
                    (None, Some(after)) => NetworkAgentHistoryDirection::After(
                        self.anchor(&session, &room, &after).await?.sequence,
                    ),
                    (None, None) => NetworkAgentHistoryDirection::Before(None),
                };
                self.history_page(&session, &room, direction, &filter, limit)
                    .await
            }
        }
    }

    async fn around(
        &self,
        session: &NetworkAgentSession,
        room: &MatrixRoomId,
        id: &str,
        before: u16,
        after: u16,
    ) -> Result<NetworkAgentRoomMessages, NetworkGatewayFailure> {
        let center = self.anchor(session, room, id).await?;
        let all = NetworkAgentHistoryFilter::default();
        let earlier = self
            .room_page(
                session,
                room,
                NetworkAgentHistoryDirection::Before(Some(center.sequence)),
                &all,
                before,
            )
            .await?;
        let later = self
            .room_page(
                session,
                room,
                NetworkAgentHistoryDirection::After(center.sequence),
                &all,
                after,
            )
            .await?;
        let messages = earlier
            .into_iter()
            .rev()
            .chain([center])
            .chain(later)
            .map(batch_preview)
            .collect();
        Ok(NetworkAgentRoomMessages {
            messages,
            next_cursor: None,
        })
    }

    /// 翻一页：多取一条，看得出后面还有没有。
    async fn history_page(
        &self,
        session: &NetworkAgentSession,
        room: &MatrixRoomId,
        direction: NetworkAgentHistoryDirection,
        filter: &NetworkAgentHistoryFilter,
        limit: u16,
    ) -> Result<NetworkAgentRoomMessages, NetworkGatewayFailure> {
        let mut page = self
            .room_page(session, room, direction, filter, limit.saturating_add(1))
            .await?;
        let more = page.len() > usize::from(limit);
        page.truncate(usize::from(limit));
        let next_cursor = more
            .then(|| page.last().map(|last| last.event_id.as_str().to_owned()))
            .flatten();
        Ok(NetworkAgentRoomMessages {
            messages: page.into_iter().map(batch_preview).collect(),
            next_cursor,
        })
    }

    async fn room_page(
        &self,
        session: &NetworkAgentSession,
        room: &MatrixRoomId,
        direction: NetworkAgentHistoryDirection,
        filter: &NetworkAgentHistoryFilter,
        limit: u16,
    ) -> Result<Vec<NetworkAgentStoredMessage>, NetworkGatewayFailure> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        self.history
            .room_messages(session.network_agent_id, room, direction, filter, limit)
            .await
            .map_err(|_| NetworkGatewayFailure::Unavailable)
    }

    /// 这个房间里的某一条（看前后的中间那条、翻页的位置）；没有就是找不到。
    async fn anchor(
        &self,
        session: &NetworkAgentSession,
        room: &MatrixRoomId,
        id: &str,
    ) -> Result<NetworkAgentStoredMessage, NetworkGatewayFailure> {
        let reference = message_ref(id).ok_or(NetworkGatewayFailure::InvalidLookup("id"))?;
        self.history
            .messages_by_id(session.network_agent_id, std::slice::from_ref(&reference))
            .await
            .map_err(|_| NetworkGatewayFailure::Unavailable)?
            .into_iter()
            .find(|message| message.room_id == *room)
            .ok_or(NetworkGatewayFailure::MessageNotFound)
    }
}

/// 事件 ID（`$` 开头）或消息 ID（UUIDv7）。
fn message_ref(id: &str) -> Option<NetworkAgentMessageRef> {
    let id = id.trim();
    if id.starts_with('$') {
        return MatrixEventId::new(id)
            .ok()
            .map(NetworkAgentMessageRef::Event);
    }
    Uuid::parse_str(id)
        .ok()
        .filter(|uuid| uuid.get_version() == Some(Version::SortRand))
        .map(|uuid| NetworkAgentMessageRef::Message(MessageId::from_uuid(uuid)))
}

fn matches(message: &NetworkAgentStoredMessage, reference: &NetworkAgentMessageRef) -> bool {
    match reference {
        NetworkAgentMessageRef::Event(event) => message.event_id == *event,
        NetworkAgentMessageRef::Message(id) => message.message_id == *id,
    }
}

/// 只看某个人：Matrix 用户 ID（`@` 开头）比 ID，否则不分大小写比名字。
fn sender(from: &str) -> Result<NetworkAgentHistorySender, NetworkGatewayFailure> {
    let from = from.trim();
    if from.is_empty() || from.len() > MESSAGE_FROM_BYTES {
        return Err(NetworkGatewayFailure::InvalidLookup("from"));
    }
    Ok(if from.starts_with('@') {
        NetworkAgentHistorySender::MatrixUserId(from.to_owned())
    } else {
        NetworkAgentHistorySender::NameFolded(from.to_lowercase())
    })
}

fn in_session(session: &NetworkAgentSession, room: &MatrixRoomId) -> bool {
    session
        .rooms
        .iter()
        .any(|joined| joined.matrix_room_id.as_str() == room.as_str())
}

/// 看前后、往前翻时和收件箱一样：长消息只给开头。
fn batch_preview(message: NetworkAgentStoredMessage) -> Value {
    let mut preview = message.preview;
    truncate_preview_value(&mut preview);
    preview
}
