//! 不说房间时看 Agent 在的所有房间（`specs/agent-identity/design.md` 第 4 步）。每个房间照旧按
//! 自己的先后和确认位置读，再按服务器收到的时间合成一页；每条本来就带房间。确认照旧按房间记。

use std::collections::HashMap;

use agent_room_application::ports::{MatrixEventId, MatrixRoomId};
use agent_room_bridge_core::messages::{MessagePreviewPage, ProjectedMessagePreview};
use agent_room_bridge_ipc::{
    IpcListPreviewsRequest, IpcMessagePreviewSummary, IpcResponse, IpcTimelineGap, limits,
    previews::{PreviewRoom, PreviewText, PreviewViewer, preview_for},
};
use agent_room_domain::messages::MessageRelation;

use super::{
    AgentRuntimeIpcFacade, BridgeAgentRuntimeSnapshot, BridgeIpcDispatchFailure,
    bounded_preview_response, invalid_request,
};

impl AgentRuntimeIpcFacade {
    /// 看所有房间：给了位置的房间从它之后，别的照 `fromAck`。挂着等时哪个房间来了都交。
    pub(super) async fn read_every_room(
        &self,
        request: IpcListPreviewsRequest,
        oldest_first: bool,
        waiting: bool,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let rooms = rooms_of(&runtime).await?;
        let mut given = request
            .room_cursors
            .iter()
            .map(|cursor| (cursor.room_id.as_str(), cursor.after_event_id.as_str()))
            .collect::<HashMap<_, _>>();
        let mut queries = Vec::with_capacity(rooms.len());
        for room in &rooms {
            let after = given
                .remove(room.as_str())
                .map(|event| MatrixEventId::new(event.to_owned()))
                .transpose()
                .map_err(|_| invalid_request("bridge.ipc.event_id_invalid"))?;
            queries.push(
                self.preview_query(
                    room,
                    after,
                    None,
                    request.from_ack,
                    oldest_first,
                    request.limit,
                )
                .await?,
            );
        }
        let typing = runtime.typing.as_ref().filter(|_| waiting);
        let pages = self
            .pages_when_ready(
                &queries,
                &runtime,
                waiting.then_some(request.wait_ms).flatten(),
                &runtime.room_id,
                typing,
            )
            .await?;
        let (picked, more) = merge(&pages, usize::from(request.limit), oldest_first);
        let (previews, gaps) = self.describe(&runtime, &picked, oldest_first).await;
        let response = bounded_preview_response(
            previews,
            more.then(|| {
                picked
                    .last()
                    .map(|preview| preview.event_id.as_str().to_owned())
            })
            .flatten(),
            typing.map_or_else(Vec::new, |typing| {
                rooms
                    .iter()
                    .flat_map(|room| typing.now(room.as_str()))
                    .collect()
            }),
            gaps,
        )?;
        self.note_host_wait(
            &runtime,
            &runtime.room_id,
            waiting && (request.keep_waiting || picked.is_empty()),
        )
        .await;
        Ok(response)
    }

    /// 合成的一页按房间取回复摘录、房间名和缺口，再照合成的先后写成给客户端的样子。
    async fn describe(
        &self,
        runtime: &BridgeAgentRuntimeSnapshot,
        picked: &[&ProjectedMessagePreview],
        oldest_first: bool,
    ) -> (Vec<IpcMessagePreviewSummary>, Vec<IpcTimelineGap>) {
        let mut by_room: Vec<(&MatrixRoomId, Vec<ProjectedMessagePreview>)> = Vec::new();
        for preview in picked {
            match by_room
                .iter_mut()
                .find(|(room, _)| *room == &preview.room_id)
            {
                Some((_, previews)) => previews.push((*preview).clone()),
                None => by_room.push((&preview.room_id, vec![(*preview).clone()])),
            }
        }
        let mut replied = HashMap::new();
        let mut contexts = HashMap::new();
        let mut gaps = Vec::new();
        for (room, previews) in &by_room {
            replied.insert(*room, self.replied_messages(room, previews).await);
            // 房间名和加入时间读不出来只是少了这两项。
            contexts.insert(
                *room,
                self.previews.room_context(room).await.unwrap_or_default(),
            );
            if oldest_first {
                gaps.extend(self.inbox_gaps(room, previews).await);
            }
        }
        let viewer = PreviewViewer {
            agent_id: runtime.identity.agent_id(),
            matrix_user_id: runtime.identity.matrix_user_id().as_str(),
        };
        let previews = picked
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
                        .and_then(|found| found.get(&id)),
                });
                // 长消息只给开头，全文按 ID 取（`GetMessages`）。
                preview_for(preview, viewer, room, target, PreviewText::Batch)
            })
            .collect();
        (previews, gaps)
    }
}

/// Agent 此刻在的房间：会话的那个在前（照旧问服务器），再加上这次连上以后同步里看到的，最多
/// [`limits::SESSION_ROOMS`] 个。被移出了会话的那个房间就只看别的；一个也不在时照旧说不在房间里。
async fn rooms_of(
    runtime: &BridgeAgentRuntimeSnapshot,
) -> Result<Vec<MatrixRoomId>, BridgeIpcDispatchFailure> {
    let mut rooms = runtime
        .room_entry
        .as_ref()
        .map(|entry| entry.joined_rooms())
        .unwrap_or_default();
    match runtime.message_room(None).await {
        Ok((session, _)) => {
            rooms.retain(|room| room != &session);
            rooms.insert(0, session);
        }
        Err(failure) if failure.code() == "bridge.room_not_joined" || rooms.is_empty() => {
            rooms.retain(|room| room != &runtime.room_id);
            if rooms.is_empty() {
                return Err(failure);
            }
        }
        // 问不到服务器时照同步里看到的读。
        Err(_) => {}
    }
    rooms.truncate(limits::SESSION_ROOMS);
    Ok(rooms)
}

/// 几个房间各自按先后的一页，按服务器收到的时间合成一页，每个房间里的先后不变。放不下的、
/// 房间里还有的，算还有下一页。
fn merge(
    pages: &[MessagePreviewPage],
    limit: usize,
    oldest_first: bool,
) -> (Vec<&ProjectedMessagePreview>, bool) {
    let mut taken = vec![0_usize; pages.len()];
    let mut picked = Vec::new();
    while picked.len() < limit {
        let next = pages
            .iter()
            .zip(&taken)
            .enumerate()
            .filter_map(|(index, (page, taken))| {
                page.previews().get(*taken).map(|preview| (index, preview))
            })
            .min_by_key(|(index, preview)| {
                let at = received_at(preview);
                (if oldest_first { at } else { u64::MAX - at }, *index)
            });
        let Some((index, preview)) = next else {
            break;
        };
        taken[index] += 1;
        picked.push(preview);
    }
    let more = pages
        .iter()
        .zip(&taken)
        .any(|(page, taken)| page.next_cursor().is_some() || *taken < page.previews().len());
    (picked, more)
}

/// 服务器收到的时间；没有时用发送方写的时间。
fn received_at(preview: &ProjectedMessagePreview) -> u64 {
    preview
        .origin_server_timestamp
        .unwrap_or_else(|| u64::try_from(preview.created_at.value()).unwrap_or_default())
}
