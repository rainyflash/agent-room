//! 分享一段对话（`specs/redesign/project-rooms.md`“分享一段对话”，3P-7a）：以建房间的应用服务账号从最新
//! 往回一页页读放着实况的房间，最多 500 条事件，读到这一段的第一句为止；同时读一次房间状态（隐藏了哪些、
//! 加没加密）。读到的经分享页实况同一套解析和验签：撤回、隐藏的不算，改过的照改后的。
//!
//! 分享的时候（[`RoomExcerptTimeline`]）只交出每一句看得见的话的事件 ID 和时间，用例拿它数几句、看连不
//! 连着。读到的话只在这一次请求里，不存。

use std::{collections::HashSet, sync::Arc};

use agent_room_application::ports::{
    AgentInstanceSignatureVerifier, AgentInstanceVerificationRepository, Clock, ExcerptMessageRef,
    ExcerptTimelineRead, ExcerptTimelineUnavailable, MatrixBackfillToken, MatrixEventId,
    MatrixFailure, MatrixRoomId, MatrixTimelineEvent, PortFuture, PublicLobbyMatrixReader,
    RoomExcerptTimeline,
};
use agent_room_bridge_core::{
    agent_verification::{
        AgentInstanceMessageAuthenticator, AgentInstanceMessageAuthenticatorDependencies,
    },
    messages::MessageProjectionMutation,
};
use agent_room_domain::time::UtcMillis;

use super::{REFRESH_DEADLINE, reading, rooms::encrypted, view};

/// 往回最多读这么多条事件（消息和修订）。
const MAX_EVENTS: usize = 500;
/// 一页最多读这么多条：聊天服务器那边一次最多给 100 条。
const PAGE_EVENTS: usize = 100;

pub(crate) struct ExcerptTimelineDependencies {
    pub(crate) reader: Arc<dyn PublicLobbyMatrixReader>,
    pub(crate) verification: Arc<dyn AgentInstanceVerificationRepository>,
    pub(crate) signatures: Arc<dyn AgentInstanceSignatureVerifier>,
    pub(crate) clock: Arc<dyn Clock>,
}

/// 往回读到第一句时读到的：消息与修订（按时间先后）和被隐藏的事件。
pub(super) struct ExcerptReading {
    pub(super) mutations: Vec<MessageProjectionMutation>,
    pub(super) hidden: HashSet<String>,
}

pub(super) enum ReadBack {
    Found(ExcerptReading),
    /// 最近 500 条事件里没有那一句。
    NotReachable,
    /// 聊天服务器上这间房加了密。
    Encrypted,
}

/// 这一次没读成，只记日志。日志里不写房间号、事件 ID。
#[derive(Debug)]
pub(super) enum ReadBackFailure {
    Matrix(MatrixFailure),
    Reading(reading::ReadingFailure),
    Timeout,
}

impl ReadBackFailure {
    pub(super) fn log(&self) {
        match self {
            Self::Matrix(failure) => tracing::warn!(
                operation = ?failure.operation(),
                failure = ?failure.kind(),
                "分享一段对话：Synapse 没读出来"
            ),
            Self::Reading(failure) => {
                tracing::warn!(failure = ?failure, "分享一段对话：读到的验不了");
            }
            Self::Timeout => tracing::warn!("分享一段对话：读得太久，这次放弃"),
        }
    }
}

/// 往回读房间的那一段。
pub(crate) struct ExcerptTimelineReader {
    reader: Arc<dyn PublicLobbyMatrixReader>,
    authenticator: Arc<AgentInstanceMessageAuthenticator>,
}

impl ExcerptTimelineReader {
    pub(crate) fn new(dependencies: ExcerptTimelineDependencies) -> Self {
        Self {
            reader: dependencies.reader,
            authenticator: Arc::new(AgentInstanceMessageAuthenticator::new(
                AgentInstanceMessageAuthenticatorDependencies {
                    verification: Arc::new(reading::CachedVerification::new(
                        dependencies.verification,
                        dependencies.clock,
                    )),
                    signatures: dependencies.signatures,
                },
            )),
        }
    }

    /// 从最新往回读到 `first_event_id` 那一句，一次最多等 8 秒。
    pub(super) async fn read_back(
        &self,
        room_id: &MatrixRoomId,
        first_event_id: &MatrixEventId,
    ) -> Result<ReadBack, ReadBackFailure> {
        tokio::time::timeout(
            REFRESH_DEADLINE,
            self.read_back_within(room_id, first_event_id),
        )
        .await
        .unwrap_or(Err(ReadBackFailure::Timeout))
    }

    async fn read_back_within(
        &self,
        room_id: &MatrixRoomId,
        first_event_id: &MatrixEventId,
    ) -> Result<ReadBack, ReadBackFailure> {
        let (events, state) =
            tokio::try_join!(self.events_back_to(room_id, first_event_id), async {
                self.reader
                    .current_state(room_id)
                    .await
                    .map_err(ReadBackFailure::Matrix)
            },)?;
        if encrypted(&state) {
            return Ok(ReadBack::Encrypted);
        }
        let Some(events) = events else {
            return Ok(ReadBack::NotReachable);
        };
        let mutations = reading::verified_messages(self.authenticator.clone(), room_id, events)
            .await
            .map_err(ReadBackFailure::Reading)?;
        Ok(ReadBack::Found(ExcerptReading {
            mutations,
            hidden: reading::hidden_events(&state),
        }))
    }

    /// 一页页往回读，读到那一句所在的那一页为止，按时间先后交出读到的全部事件。读满 500 条、读到房间的
    /// 开头都没见到那一句是 `None`。
    async fn events_back_to(
        &self,
        room_id: &MatrixRoomId,
        first_event_id: &MatrixEventId,
    ) -> Result<Option<Vec<MatrixTimelineEvent>>, ReadBackFailure> {
        let mut pages = Vec::new();
        let mut read = 0;
        let mut from: Option<MatrixBackfillToken> = None;
        loop {
            let limit = u16::try_from(PAGE_EVENTS.min(MAX_EVENTS - read)).unwrap_or(1);
            let page = self
                .reader
                .messages_before(room_id, from.as_ref(), limit)
                .await
                .map_err(ReadBackFailure::Matrix)?;
            let found = page
                .events
                .iter()
                .any(|event| event.event_id() == Some(first_event_id));
            read += page.read;
            pages.push(page.events);
            if found {
                break;
            }
            match page.next {
                Some(next) if page.read > 0 && read < MAX_EVENTS => from = Some(next),
                _ => return Ok(None),
            }
        }
        // 先读到的是新的那一页：倒过来接起来，按时间先后。
        pages.reverse();
        Ok(Some(pages.concat()))
    }
}

impl RoomExcerptTimeline for ExcerptTimelineReader {
    fn visible_messages<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        first_event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, Result<ExcerptTimelineRead, ExcerptTimelineUnavailable>> {
        Box::pin(async move {
            match self.read_back(room_id, first_event_id).await {
                Ok(ReadBack::Found(reading)) => Ok(ExcerptTimelineRead::Found(message_refs(
                    &reading.mutations,
                    &reading.hidden,
                ))),
                Ok(ReadBack::NotReachable) => Ok(ExcerptTimelineRead::NotReachable),
                Ok(ReadBack::Encrypted) => {
                    tracing::warn!("分享一段对话：目录记着不加密，聊天服务器上却加了密，不放");
                    Ok(ExcerptTimelineRead::Encrypted)
                }
                Err(failure) => {
                    failure.log();
                    Err(ExcerptTimelineUnavailable)
                }
            }
        })
    }
}

/// 每一句看得见的话的事件 ID 和时间，按时间先后。
fn message_refs(
    mutations: &[MessageProjectionMutation],
    hidden: &HashSet<String>,
) -> Vec<ExcerptMessageRef> {
    view::all_visible_messages(mutations, hidden)
        .iter()
        .filter_map(|message| {
            Some(ExcerptMessageRef {
                event_id: message.original.event_id.clone(),
                sent_at: UtcMillis::new(view::sent_at(message)).ok()?,
            })
        })
        .collect()
}
