//! 按需查看（`specs/agent-reading/design.md` 第 3 步）：按 ID 取、看前后、往前翻。
//!
//! 都不动收件箱的位置，只给这个 Agent 所在房间里的消息。按 ID 取给全文；看前后、往前翻和收件箱
//! 一样，长消息只给开头，要全文再按 ID 取。

use std::collections::HashMap;

use agent_room_application::ports::{MatrixEventId, MatrixRoomId};
use agent_room_bridge_core::messages::{
    MessageLookupId, MessagePreviewPage, MessagePreviewQuery, MessageRoomContext,
    ProjectedMessagePreview,
};
use agent_room_bridge_ipc::previews::{
    PreviewRoom, PreviewText, PreviewViewer, actor_user_and_name, preview_for,
};
use agent_room_bridge_ipc::{
    IpcErrorCategory, IpcGetMessagesRequest, IpcMessagePreviewSummary, IpcMessagesAroundRequest,
    IpcMethod, IpcResponse, IpcRoomHistoryRequest,
};
use agent_room_domain::{ids::MessageId, messages::MessageRelation};
use uuid::Uuid;

use super::{
    AgentRuntimeIpcFacade, BridgeAgentRuntimeSnapshot, BridgeIpcDispatchFailure,
    PREVIEW_PAGE_BYTES, invalid_request, map_preview_query_failure,
};

/// 往前翻带过滤时，一次最多看这么多条就先交，免得在很长的房间里一次查到底；没看完的给接着翻的位置。
const HISTORY_SCAN_LIMIT: usize = 500;
/// 带过滤往前翻时，每次从本地消息库取这么多条来挑。
const HISTORY_SCAN_PAGE: u16 = 50;

impl AgentRuntimeIpcFacade {
    /// 按需查看的三个方法从这里进来。
    pub(in crate::ipc) async fn view(
        &self,
        method: IpcMethod,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        match method {
            IpcMethod::GetMessages(request) => self.get_messages(request).await,
            IpcMethod::MessagesAround(request) => self.messages_around(request).await,
            IpcMethod::RoomHistory(request) => self.room_history(request).await,
            _ => Err(invalid_request("bridge.ipc.method_invalid")),
        }
    }

    /// 按 ID 取：事件 ID 或消息 ID，跨房间也行，给全文，按要的顺序。
    async fn get_messages(
        &self,
        request: IpcGetMessagesRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let wanted = request
            .ids
            .iter()
            .map(|id| lookup_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        let found = self
            .previews
            .lookup_messages(&wanted)
            .await
            .map_err(map_preview_query_failure)?;
        let visible = visible_messages(&runtime, found).await?;
        // 按要的顺序；同一条用事件 ID 和消息 ID 各要了一次，也只给一次。
        let mut ordered: Vec<ProjectedMessagePreview> = Vec::new();
        let mut missing = Vec::new();
        for (raw, id) in request.ids.iter().zip(&wanted) {
            match visible.iter().find(|message| matches_id(message, id)) {
                Some(message) => {
                    if !ordered
                        .iter()
                        .any(|seen| seen.message_id == message.message_id)
                    {
                        ordered.push(message.clone());
                    }
                }
                None => missing.push(raw.clone()),
            }
        }
        let summaries = self.summaries(&runtime, &ordered, PreviewText::Full).await;
        let (messages, more) = fit_in_reply(summaries);
        Ok(IpcResponse::Messages {
            messages,
            missing,
            more,
        })
    }

    /// 看前后：某一条和它前后各几条，早的在前。
    async fn messages_around(
        &self,
        request: IpcMessagesAroundRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let (room_id, _) = runtime.message_room(request.room_id).await?;
        let anchor = lookup_id(&request.id)?;
        let around = self
            .previews
            .messages_around(&room_id, &anchor, request.before, request.after)
            .await
            .map_err(map_preview_query_failure)?;
        let Some(center) = around.anchor else {
            return Err(message_not_found());
        };
        let center_index = around.before.len();
        let mut ordered = around.before;
        ordered.push(center);
        ordered.extend(around.after);
        let summaries = self.summaries(&runtime, &ordered, PreviewText::Batch).await;
        Ok(IpcResponse::RoomMessages {
            messages: trim_around(summaries, center_index),
            next_cursor: None,
        })
    }

    /// 往前翻（给了 `after` 就往后翻），可以只看某个人、只看提到我的。
    async fn room_history(
        &self,
        request: IpcRoomHistoryRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let (room_id, _) = runtime.message_room(request.room_id.clone()).await?;
        let forward = request.after.is_some();
        let mut cursor = match request.before.as_deref().or(request.after.as_deref()) {
            Some(id) => Some(self.cursor_event(&room_id, id).await?),
            None => None,
        };
        let limit = usize::from(request.limit);
        let filtered = request.from.is_some() || request.mentions_me;
        let mut picked: Vec<IpcMessagePreviewSummary> = Vec::new();
        let mut scanned = 0;
        let mut next_cursor = None;
        loop {
            let want = if filtered {
                HISTORY_SCAN_PAGE
            } else {
                u16::try_from(limit - picked.len()).unwrap_or(HISTORY_SCAN_PAGE)
            };
            let page = self
                .history_page(&room_id, cursor.take(), forward, want)
                .await?;
            let summaries = self
                .summaries(&runtime, page.previews(), PreviewText::Batch)
                .await;
            let last = summaries.len().saturating_sub(1);
            let more_in_store = page.next_cursor().is_some();
            for (index, summary) in summaries.into_iter().enumerate() {
                scanned += 1;
                let event_id = summary.event_id.clone();
                let more_after_this = index < last || more_in_store;
                if wanted_by(&summary, &request) {
                    picked.push(summary);
                    if picked.len() == limit {
                        next_cursor = more_after_this.then_some(event_id);
                        break;
                    }
                }
                if scanned >= HISTORY_SCAN_LIMIT {
                    next_cursor = more_after_this.then_some(event_id);
                    break;
                }
            }
            if picked.len() == limit || scanned >= HISTORY_SCAN_LIMIT {
                break;
            }
            match page.next_cursor() {
                Some(next) => cursor = Some(next.clone()),
                None => break,
            }
        }
        let (messages, cut_at) = fit_page(picked);
        Ok(IpcResponse::RoomMessages {
            messages,
            next_cursor: cut_at.or(next_cursor),
        })
    }

    async fn history_page(
        &self,
        room_id: &MatrixRoomId,
        cursor: Option<MatrixEventId>,
        forward: bool,
        limit: u16,
    ) -> Result<MessagePreviewPage, BridgeIpcDispatchFailure> {
        let query = match (cursor, forward) {
            (Some(cursor), true) => MessagePreviewQuery::after(room_id.clone(), cursor, limit),
            (cursor, _) => MessagePreviewQuery::new(room_id.clone(), cursor, limit),
        }
        .map_err(|_| invalid_request("bridge.ipc.preview_limit_invalid"))?;
        self.previews
            .list_previews(&query)
            .await
            .map_err(map_preview_query_failure)
    }

    /// 翻页的位置：事件 ID 直接用；消息 ID 先在这个房间里找到它的事件 ID。
    async fn cursor_event(
        &self,
        room_id: &MatrixRoomId,
        id: &str,
    ) -> Result<MatrixEventId, BridgeIpcDispatchFailure> {
        match lookup_id(id)? {
            MessageLookupId::Event(event) => Ok(event),
            MessageLookupId::Message(message) => self
                .previews
                .find_messages(room_id, &[message])
                .await
                .map_err(map_preview_query_failure)?
                .into_iter()
                .next()
                .map(|found| found.event_id)
                .ok_or_else(message_not_found),
        }
    }

    /// 一组消息交给这个 Agent 看：标出自己发的、提到它的、加入之前的，附上房间名和被回复那条的开头。
    /// 按 ID 取时消息可能来自几个房间，房间信息和被回复的消息按房间分别取。
    async fn summaries(
        &self,
        runtime: &BridgeAgentRuntimeSnapshot,
        messages: &[ProjectedMessagePreview],
        text: PreviewText,
    ) -> Vec<IpcMessagePreviewSummary> {
        let mut contexts: HashMap<MatrixRoomId, MessageRoomContext> = HashMap::new();
        let mut replied: HashMap<MatrixRoomId, HashMap<MessageId, ProjectedMessagePreview>> =
            HashMap::new();
        for message in messages {
            if contexts.contains_key(&message.room_id) {
                continue;
            }
            // 房间名和加入时间读不出来只是少了这两项。
            let context = self
                .previews
                .room_context(&message.room_id)
                .await
                .unwrap_or_default();
            let in_room: Vec<ProjectedMessagePreview> = messages
                .iter()
                .filter(|other| other.room_id == message.room_id)
                .cloned()
                .collect();
            replied.insert(
                message.room_id.clone(),
                self.replied_messages(&message.room_id, &in_room).await,
            );
            contexts.insert(message.room_id.clone(), context);
        }
        let viewer = PreviewViewer {
            agent_id: runtime.identity.agent_id(),
            matrix_user_id: runtime.identity.matrix_user_id().as_str(),
        };
        messages
            .iter()
            .map(|preview| {
                let context = contexts.get(&preview.room_id);
                let room = PreviewRoom {
                    name: context.and_then(|context| context.name.as_deref()),
                    joined_at_ms: context.and_then(|context| context.joined_at_ms),
                };
                let target = preview.relation.and_then(|relation| match relation {
                    MessageRelation::ReplyTo(id) => replied
                        .get(&preview.room_id)
                        .and_then(|in_room| in_room.get(&id)),
                });
                preview_for(preview, viewer, room, target, text)
            })
            .collect()
    }
}

/// 只留这个 Agent 所在房间里的；一个房间只问一次。不在那个房间的当作找不到，查不了的照实报错。
async fn visible_messages(
    runtime: &BridgeAgentRuntimeSnapshot,
    found: Vec<ProjectedMessagePreview>,
) -> Result<Vec<ProjectedMessagePreview>, BridgeIpcDispatchFailure> {
    let mut rooms: HashMap<MatrixRoomId, bool> = HashMap::new();
    let mut visible = Vec::new();
    for message in found {
        let allowed = if let Some(allowed) = rooms.get(&message.room_id) {
            *allowed
        } else {
            let allowed = match runtime
                .message_room(Some(message.room_id.as_str().to_owned()))
                .await
            {
                Ok(_) => true,
                Err(failure) if failure.code == "bridge.room_not_joined" => false,
                Err(failure) => return Err(failure),
            };
            rooms.insert(message.room_id.clone(), allowed);
            allowed
        };
        if allowed {
            visible.push(message);
        }
    }
    Ok(visible)
}

/// 事件 ID（`$` 开头）或消息 ID（UUIDv7）。IPC 已经校验过格式。
pub(super) fn lookup_id(id: &str) -> Result<MessageLookupId, BridgeIpcDispatchFailure> {
    if id.starts_with('$') {
        MatrixEventId::new(id)
            .map(MessageLookupId::Event)
            .map_err(|_| invalid_request("bridge.ipc.message_id_invalid"))
    } else {
        Uuid::parse_str(id)
            .map(|uuid| MessageLookupId::Message(MessageId::from_uuid(uuid)))
            .map_err(|_| invalid_request("bridge.ipc.message_id_invalid"))
    }
}

fn matches_id(message: &ProjectedMessagePreview, id: &MessageLookupId) -> bool {
    match id {
        MessageLookupId::Event(event) => message.event_id == *event,
        MessageLookupId::Message(message_id) => message.message_id == *message_id,
    }
}

/// 往前翻时要不要这一条：只看某个人（Matrix 用户 ID，或者不分大小写的名字）、只看提到我的。
fn wanted_by(summary: &IpcMessagePreviewSummary, request: &IpcRoomHistoryRequest) -> bool {
    if request.mentions_me && !summary.mentions_me {
        return false;
    }
    let Some(from) = request.from.as_deref().map(str::trim) else {
        return true;
    };
    let (matrix_user_id, display_name) = actor_user_and_name(&summary.actor);
    if from.starts_with('@') {
        matrix_user_id == from
    } else {
        display_name.to_lowercase() == from.to_lowercase()
    }
}

pub(super) fn message_not_found() -> BridgeIpcDispatchFailure {
    BridgeIpcDispatchFailure::new(
        "bridge.message_not_found",
        IpcErrorCategory::Validation,
        false,
    )
}

fn encoded_size(summary: &IpcMessagePreviewSummary) -> usize {
    serde_json::to_vec(summary).map_or(usize::MAX, |bytes| bytes.len() + 1)
}

/// 按 ID 取：一次回复放不下的（一条就可能有十几 KB），留给下一次，`more` 里是它们的事件 ID。
fn fit_in_reply(
    summaries: Vec<IpcMessagePreviewSummary>,
) -> (Vec<IpcMessagePreviewSummary>, Vec<String>) {
    let mut bytes = 0;
    let mut kept = Vec::new();
    let mut more = Vec::new();
    for summary in summaries {
        let size = encoded_size(&summary);
        if more.is_empty() && (kept.is_empty() || bytes + size <= PREVIEW_PAGE_BYTES) {
            bytes += size;
            kept.push(summary);
        } else {
            more.push(summary.event_id);
        }
    }
    (kept, more)
}

/// 往前翻的一页放不下时截在放得下的地方，接着翻的位置改成最后放进去的那条。
fn fit_page(
    summaries: Vec<IpcMessagePreviewSummary>,
) -> (Vec<IpcMessagePreviewSummary>, Option<String>) {
    let mut bytes = 0;
    let mut kept: Vec<IpcMessagePreviewSummary> = Vec::new();
    for summary in summaries {
        let size = encoded_size(&summary);
        if !kept.is_empty() && bytes + size > PREVIEW_PAGE_BYTES {
            let cut_at = kept.last().map(|last| last.event_id.clone());
            return (kept, cut_at);
        }
        bytes += size;
        kept.push(summary);
    }
    (kept, None)
}

/// 看前后放不下时，从离中间那条最远的一头开始去掉，中间那条一定留着。
fn trim_around(
    mut summaries: Vec<IpcMessagePreviewSummary>,
    mut center: usize,
) -> Vec<IpcMessagePreviewSummary> {
    let mut total: usize = summaries.iter().map(encoded_size).sum();
    while total > PREVIEW_PAGE_BYTES && summaries.len() > 1 {
        let last = summaries.len() - 1;
        let removed = if center > 0 && center >= last - center {
            center -= 1;
            summaries.remove(0)
        } else {
            summaries.remove(last)
        };
        total -= encoded_size(&removed);
    }
    summaries
}
