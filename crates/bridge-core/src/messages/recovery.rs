//! 找回房间密钥后，把之前解不开、隔离着的消息重新读出来，放回消息记录里原来的位置。
//!
//! 隔离时存储已经按观察顺序给这些事件预留了位置。这里按事件 ID 重新读取（读的时候解密），
//! 走和同步相同的校验，通过的写进预留的位置。它们排在 Agent 已经读过的消息之间，收件箱不会
//! 把几天前的话当成刚收到的再推一遍。等应答的请求只记在内存里，所以 Bridge 启动后还要对仍然
//! 解不开的事件重新请求一次房间密钥。设计见 `specs/room-key-recovery/pre-join-history.md`。

use std::collections::BTreeMap;

use agent_room_application::ports::{
    MatrixEventId, MatrixGateway, MatrixResult, MatrixRoomId, MatrixTimelineEvent, MatrixUserId,
    PortFuture,
};

use super::{
    MessageRecoveryBatch, MessageSyncFailure, MessageSyncIssueReason, MessageSyncService,
    backfill::is_transient,
};

/// 每次从存储取多少条要重读的事件。
const PAGE_EVENTS: u16 = 100;
/// 一轮最多重读多少条，剩下的下一轮接着读。
const MAX_EVENTS_PER_ROUND: usize = 1_000;
/// 启动时最多为多少个会话重新请求房间密钥（最近的优先）。
const MAX_REREQUESTED_SESSIONS: u16 = 500;

/// 重读隔离事件、请求重发房间密钥的能力；Matrix 网关天然具备，测试只需实现这两个方法。
pub trait MessageRecoverySource: Send + Sync {
    fn fetch_event<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, MatrixResult<MatrixTimelineEvent>>;

    fn request_room_keys(
        &self,
        room_id: &MatrixRoomId,
        sender: &MatrixUserId,
        sender_device: Option<&str>,
        session_ids: &[String],
    );
}

impl<T: MatrixGateway + ?Sized> MessageRecoverySource for T {
    fn fetch_event<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, MatrixResult<MatrixTimelineEvent>> {
        MatrixGateway::fetch_event(self, room_id, event_id)
    }

    fn request_room_keys(
        &self,
        room_id: &MatrixRoomId,
        sender: &MatrixUserId,
        sender_device: Option<&str>,
        session_ids: &[String],
    ) {
        MatrixGateway::request_room_keys(self, room_id, sender, sender_device, session_ids);
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MessageRecoveryOutcome {
    /// 重读出来、写回原来位置的消息。
    pub recovered_events: usize,
    /// 重读后仍然解不开或读不到、继续隔离的事件。
    pub still_isolated: usize,
    /// 解开了但因别的原因不收（比如签名不对）的事件。
    pub rejected_events: usize,
    /// 暂时读不到或这一轮读满了、下一轮接着读的会话。
    pub deferred: Vec<(MatrixRoomId, String)>,
}

enum RoomRecovery {
    Finished,
    /// 连接暂时不可用，或这一轮读满了。
    Deferred,
}

impl MessageSyncService {
    /// 重读用到这些新导入会话的隔离事件，读出来的写回原来的位置。
    ///
    /// # Errors
    ///
    /// 投影存储或验签服务不可用时返回错误；已写入的保留，其余留到下一轮。
    pub async fn recover_isolated<S: MessageRecoverySource + ?Sized>(
        &self,
        source: &S,
        sessions: &[(MatrixRoomId, String)],
    ) -> Result<MessageRecoveryOutcome, MessageSyncFailure> {
        let mut by_room = BTreeMap::<&MatrixRoomId, Vec<String>>::new();
        for (room_id, session_id) in sessions {
            let session_ids = by_room.entry(room_id).or_default();
            if !session_ids.contains(session_id) {
                session_ids.push(session_id.clone());
            }
        }
        let mut outcome = MessageRecoveryOutcome::default();
        let mut budget = MAX_EVENTS_PER_ROUND;
        for (room_id, session_ids) in by_room {
            let recovery = if budget == 0 {
                RoomRecovery::Deferred
            } else {
                self.recover_room(source, room_id, &session_ids, &mut budget, &mut outcome)
                    .await?
            };
            if matches!(recovery, RoomRecovery::Deferred) {
                outcome.deferred.extend(
                    session_ids
                        .into_iter()
                        .map(|session_id| (room_id.clone(), session_id)),
                );
            }
        }
        Ok(outcome)
    }

    /// Bridge 启动后，对存储里仍然解不开的消息重新请求一次房间密钥。返回请求的会话数。
    ///
    /// # Errors
    ///
    /// 投影存储不可用时返回错误。
    pub async fn rerequest_isolated<S: MessageRecoverySource + ?Sized>(
        &self,
        source: &S,
    ) -> Result<usize, MessageSyncFailure> {
        let sessions = self
            .projections
            .undecryptable_sessions(MAX_REREQUESTED_SESSIONS)
            .await
            .map_err(MessageSyncFailure::projection_store)?;
        let mut targets =
            BTreeMap::<(&MatrixRoomId, &MatrixUserId, Option<&str>), Vec<String>>::new();
        for isolated in &sessions {
            targets
                .entry((
                    &isolated.room_id,
                    &isolated.session.sender,
                    isolated.session.sender_device.as_deref(),
                ))
                .or_default()
                .push(isolated.session.session_id.clone());
        }
        for ((room_id, sender, sender_device), session_ids) in &targets {
            source.request_room_keys(room_id, sender, *sender_device, session_ids);
        }
        Ok(sessions.len())
    }

    async fn recover_room<S: MessageRecoverySource + ?Sized>(
        &self,
        source: &S,
        room_id: &MatrixRoomId,
        session_ids: &[String],
        budget: &mut usize,
        outcome: &mut MessageRecoveryOutcome,
    ) -> Result<RoomRecovery, MessageSyncFailure> {
        let mut after = 0;
        loop {
            let page = self
                .projections
                .undecryptable_events(room_id, session_ids, after, PAGE_EVENTS)
                .await
                .map_err(MessageSyncFailure::projection_store)?;
            let Some(last) = page.last() else {
                return Ok(RoomRecovery::Finished);
            };
            after = last.position;
            let mut events = Vec::new();
            let mut interrupted = false;
            for isolated in page.iter().take(*budget) {
                *budget -= 1;
                match source.fetch_event(room_id, &isolated.event_id).await {
                    Ok(event) => events.push(event),
                    Err(failure) if is_transient(failure) => {
                        interrupted = true;
                        break;
                    }
                    // 读不到了（比如已被删除，或已经离开房间）：留着隔离记录。
                    Err(_) => outcome.still_isolated += 1,
                }
            }
            self.write_recovered(room_id, &events, outcome).await?;
            if interrupted || *budget == 0 {
                return Ok(RoomRecovery::Deferred);
            }
            if page.len() < usize::from(PAGE_EVENTS) {
                return Ok(RoomRecovery::Finished);
            }
        }
    }

    /// 重读到的事件走和同步相同的校验：通过的写回原位，仍解不开的不动，别的原因不收的改记原因。
    async fn write_recovered(
        &self,
        room_id: &MatrixRoomId,
        events: &[MatrixTimelineEvent],
        outcome: &mut MessageRecoveryOutcome,
    ) -> Result<(), MessageSyncFailure> {
        let mut recovered = Vec::new();
        let mut issues = Vec::new();
        let mut dismissed = Vec::new();
        for event in events {
            let before = (recovered.len(), issues.len());
            self.project_room_events(
                room_id,
                std::slice::from_ref(event),
                &mut recovered,
                &mut issues,
            )
            .await?;
            if before == (recovered.len(), issues.len())
                && let Some(event_id) = event.event_id()
            {
                dismissed.push(event_id.clone());
            }
        }
        let (still_undecryptable, reclassified): (Vec<_>, Vec<_>) = issues
            .into_iter()
            .partition(|issue| issue.reason == MessageSyncIssueReason::Undecryptable);
        outcome.recovered_events += recovered.len();
        outcome.still_isolated += still_undecryptable.len();
        outcome.rejected_events += reclassified.len();
        if recovered.is_empty() && reclassified.is_empty() && dismissed.is_empty() {
            return Ok(());
        }
        self.projections
            .apply_recovery(&MessageRecoveryBatch::new(
                room_id.clone(),
                recovered,
                reclassified,
                dismissed,
            ))
            .await
            .map_err(MessageSyncFailure::projection_store)
    }
}
