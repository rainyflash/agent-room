use crate::{
    DeliveryRecord, DeliveryStage, HostReply, ReceiverBinding, ReceiverEvent, ReceiverStart,
    ReceiverState, ReceiverStore, ReceptionFailure as Failure, ReceptionResult as Result, call,
    scoped,
};
use agent_room_agent_client::{
    BridgeToolClient, BridgeToolFailure, InboxWaiter, MessageWait, WokenBatch,
    reception::ReceptionCheckpoint,
};
use agent_room_bridge_ipc::{
    IpcAckInboxRequest, IpcActorSummary, IpcBridgeState, IpcCloseHostSessionRequest,
    IpcGetMessagesRequest, IpcHostSessionState, IpcMessagePreviewSummary, IpcMethod, IpcResponse,
    IpcRoomKind, IpcSelfSummary, limits,
};
use std::{path::Path, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverMode {
    Listen,
    VerifyReceipt,
}

pub struct ReceiverContext<'a> {
    pub mode: ReceiverMode,
    pub host: &'a dyn crate::HostRunner,
    pub backend: &'a dyn BridgeToolClient,
    pub data_root: &'a Path,
    pub service: &'a str,
    pub emit: &'a (dyn Fn(ReceiverEvent) -> Result<()> + Send + Sync),
}

/// Run one explicitly bound task. Cancelling waits for session teardown; uncertain
/// deliveries remain pending and are reconciled before any subsequent host turn.
/// # Errors
/// Invalid bindings, host failures, missing replies or unavailable storage are visible.
pub async fn run(
    context: ReceiverContext<'_>,
    task_id: &str,
    stop: impl Future<Output = ()>,
) -> Result<()> {
    let store = ReceiverStore::open(context.data_root, task_id)?;
    let mut state = store
        .load()?
        .ok_or_else(|| Failure::local("receiver.state_missing"))?;
    if state.bridge_service != context.service {
        return Err(Failure::validation("receiver.bridge_service_changed"));
    }
    if context.mode == ReceiverMode::Listen {
        state.binding.host.validate()?;
    }
    if state
        .execution
        .as_ref()
        .is_some_and(|execution| execution.releasing)
    {
        crate::execution::release(context.backend, &store, &mut state).await?;
    }
    let mut session_id = None;
    let result = tokio::select! {
        biased;
        () = stop => Ok(()),
        result = receive_loop(&context, &store, &mut state, &mut session_id) => result,
    };
    let mut close = if let Some(session_id) = session_id {
        call(
            context.backend,
            IpcMethod::CloseHostSession(IpcCloseHostSessionRequest { session_id }),
        )
        .await
        .map(|_| ())
    } else {
        Ok(())
    };
    if close.is_ok() {
        close = crate::execution::release(context.backend, &store, &mut state).await;
    }
    if let Err(mut error) = result {
        if let Err(close_error) = close {
            error.details.insert("closeError".into(), close_error.code);
        }
        return Err(error);
    }
    close.map_err(|cause| {
        let mut error = Failure::local("receiver.release_unconfirmed");
        error.details.insert("closeError".into(), cause.code);
        error
    })?;
    (context.emit)(ReceiverEvent::Stopped)
}

async fn receive_loop(
    context: &ReceiverContext<'_>,
    store: &ReceiverStore,
    state: &mut ReceiverState,
    session_id: &mut Option<String>,
) -> Result<()> {
    let mut delay = 1;
    loop {
        match receive_connected(context, store, state, session_id).await {
            Err(error) if error.code == "reception.handoff_requested" => {
                state.enabled = false;
                store.save(state)?;
                return Ok(());
            }
            Err(error) if error.retryable => {
                // Draining the previous session also covers outbound IPC calls that
                // survived cancellation of a host turn or a network request.
                if let Some(id) = session_id.as_ref() {
                    call(
                        context.backend,
                        IpcMethod::CloseHostSession(IpcCloseHostSessionRequest {
                            session_id: id.clone(),
                        }),
                    )
                    .await?;
                    *session_id = None;
                }
                (context.emit)(ReceiverEvent::Reconnecting {
                    error,
                    retry_in_seconds: delay,
                })?;
                tokio::time::sleep(Duration::from_secs(delay)).await;
                delay = (delay * 2).min(30);
            }
            result => return result,
        }
    }
}

async fn receive_connected(
    context: &ReceiverContext<'_>,
    store: &ReceiverStore,
    state: &mut ReceiverState,
    session_id: &mut Option<String>,
) -> Result<()> {
    let binding = state.binding.clone();
    let IpcResponse::HostSession { session } = call(
        context.backend,
        IpcMethod::OpenHostSession(binding.session.clone()),
    )
    .await?
    else {
        return Err(Failure::local("receiver.response_invalid"));
    };
    *session_id = Some(session.session_id.clone());
    if matches!(
        session.state,
        IpcHostSessionState::Failed | IpcHostSessionState::Closed
    ) {
        return Err(Failure::local(
            session
                .error_code
                .as_deref()
                .unwrap_or("receiver.session_failed"),
        ));
    }
    let summary = wait_until_ready(context.backend, &session.session_id).await?;
    state.room_catalog_id.clone_from(&summary.room_catalog_id);
    state.instance_id = Some(summary.instance_id.clone());
    if let Some(agent_id) = &state.agent_id {
        if agent_id != &summary.agent.agent_id {
            return Err(Failure::local("receiver.identity_changed"));
        }
    } else {
        state.checkpoint = ReceptionCheckpoint::Ready {
            after_event_id: initial_cursor(context.backend, &binding, &session.session_id).await?,
        };
        state.agent_id = Some(summary.agent.agent_id.clone());
    }
    store.save(state)?;
    crate::execution::claim(context.backend, &session.session_id, store, state).await?;
    if matches!(state.checkpoint, ReceptionCheckpoint::Pending { .. }) {
        reconcile(
            context,
            store,
            state,
            &session.session_id,
            &summary.agent.agent_id,
        )
        .await?;
    } else if context.mode == ReceiverMode::VerifyReceipt {
        return Err(Failure::validation("receiver.no_pending_delivery"));
    }
    if context.mode == ReceiverMode::VerifyReceipt {
        return Ok(());
    }
    (context.emit)(ReceiverEvent::Ready {
        agent_id: summary.agent.agent_id.clone(),
        host_task_id: binding.host.task_id.clone(),
        room_id: binding.policy.room_id.clone(),
    })?;
    let heartbeat =
        crate::execution::request(state, agent_room_bridge_ipc::ReceptionCommand::Heartbeat)?;
    tokio::select! {
        result = crate::execution::monitor(context.backend, &session.session_id, heartbeat) => result,
        result = poll_inbox(context, store, state, &summary, &session.session_id) => result,
    }
}

/// 按后台回复的规则等（`specs/agent-reading/waiting.md`「后台回复」）：主人说的、跟它有关的话，
/// 和私人房间里点名或回复它的，防抖以后合成一批交给宿主。
async fn poll_inbox(
    context: &ReceiverContext<'_>,
    store: &ReceiverStore,
    state: &mut ReceiverState,
    summary: &IpcSelfSummary,
    session_id: &str,
) -> Result<()> {
    let private_room = private_room(context.backend, state).await;
    loop {
        let policy = state.binding.policy.clone();
        let rules = policy.wait_rules();
        let mut waiter = InboxWaiter::new(
            session_id.to_owned(),
            Some(policy.room_id.clone()),
            state.checkpoint.cursor().map(str::to_owned),
            50,
            rules,
        )
        .with_wakes(move |message| policy.wakes(message, private_room));
        let batch = waiter
            .next(context.backend, MessageWait::UntilMessage)
            .await
            .map_err(session_failure)?;
        tokio::task::yield_now().await;
        deliver(context, store, state, summary, session_id, &batch).await?;
    }
}

/// 会话断了或者被关了，重连就好。
fn session_failure(error: BridgeToolFailure) -> Failure {
    let mut failure = Failure::from(error);
    if matches!(
        failure.code.as_str(),
        "bridge.host_session.not_found" | "bridge.host_session.closed"
    ) {
        failure.retryable = true;
    }
    failure
}

/// 它在不在私人房间里：私人房间里别人点名它也叫得醒，公开大厅里只认主人。看不出来就当公开大厅。
async fn private_room(backend: &dyn BridgeToolClient, state: &ReceiverState) -> bool {
    let Ok(IpcResponse::Rooms { rooms }) = call(backend, IpcMethod::ListRooms).await else {
        return false;
    };
    rooms.iter().any(|room| {
        room.kind == IpcRoomKind::PrivateRoom
            && (room.matrix_room_id.as_deref() == Some(state.binding.policy.room_id.as_str())
                || state.room_catalog_id.as_deref() == Some(room.catalog_id.as_str()))
    })
}

/// 回复挂在哪条下面：叫醒它的最后一条；定时看一眼时是最新的一条人说的话，没有就是最新的一条。
fn reply_target(batch: &WokenBatch) -> Option<String> {
    batch
        .wake
        .event_ids
        .iter()
        .rev()
        .find_map(|event_id| {
            batch
                .previews
                .iter()
                .find(|message| message.event_id == *event_id)
        })
        .or_else(|| {
            batch
                .previews
                .iter()
                .rev()
                .find(|message| matches!(message.actor, IpcActorSummary::Human { .. }))
        })
        .or_else(|| batch.previews.last())
        .map(|message| message.message_id.clone())
}

/// 收件箱里超过 1000 字的消息只给开头；交给宿主之前按 ID 取回全文，宿主照旧读到整条。
/// 取不回来的留着开头（带 `truncated`），宿主还能用只读工具按 ID 去取。
async fn with_full_text(
    backend: &dyn BridgeToolClient,
    session_id: &str,
    messages: &[IpcMessagePreviewSummary],
) -> Vec<IpcMessagePreviewSummary> {
    let mut messages = messages.to_vec();
    let mut pending: Vec<String> = messages
        .iter()
        .filter(|message| {
            message
                .conversation
                .as_ref()
                .is_some_and(|chat| chat.truncated)
        })
        .map(|message| message.event_id.clone())
        .collect();
    while !pending.is_empty() {
        let ids: Vec<String> = pending
            .drain(..pending.len().min(limits::MESSAGE_LOOKUP_IDS))
            .collect();
        let Ok(IpcResponse::Messages {
            messages: whole,
            more,
            ..
        }) = call(
            backend,
            scoped(
                session_id,
                IpcMethod::GetMessages(IpcGetMessagesRequest { ids }),
            ),
        )
        .await
        else {
            break;
        };
        // 一条都没取回来就不再问，免得原地打转。
        if whole.is_empty() {
            break;
        }
        for message in whole {
            if let Some(slot) = messages
                .iter_mut()
                .find(|slot| slot.event_id == message.event_id)
            {
                *slot = message;
            }
        }
        // 这次放不下的下一次先取。
        let mut next = more;
        next.append(&mut pending);
        pending = next;
    }
    messages
}

async fn deliver(
    context: &ReceiverContext<'_>,
    store: &ReceiverStore,
    state: &mut ReceiverState,
    summary: &IpcSelfSummary,
    session_id: &str,
    batch: &WokenBatch,
) -> Result<()> {
    let anchor = batch
        .cursor
        .clone()
        .ok_or_else(|| Failure::local("receiver.batch_invalid"))?;
    let target = reply_target(batch).ok_or_else(|| Failure::local("receiver.batch_invalid"))?;
    if recover_previous(context, store, state, summary, session_id).await? {
        return Ok(());
    }
    begin_batch(state, store, &anchor, &target)?;
    // Persist the original submission on the server before a model can run.
    crate::execution::save(context.backend, Some(session_id), store, state).await?;
    emit_delivery(context, state)?;
    let record = state
        .last_delivery
        .clone()
        .ok_or_else(|| Failure::local("receiver.delivery_missing"))?;
    change_stage(state, store, DeliveryStage::Running)?;
    emit_delivery(context, state)?;
    let messages = with_full_text(context.backend, session_id, &batch.previews).await;
    let host = context
        .host
        .resume(crate::HostDelivery {
            binding: &state.binding.host,
            data_root: context.data_root,
            service: context.service,
            session_id,
            submission_id: &record.submission_id,
            messages: &messages,
            wake: &batch.wake,
            reply_to: &record.message_id,
            skipped: batch.skipped,
        })
        .await;
    let sent = match host {
        Ok(reply) if reply.is_silent() => {
            return finish_silently(context, store, state, session_id, &anchor).await;
        }
        Ok(reply) => send_reply(context, state, session_id, reply, &record).await,
        Err(error) => Err(error),
    };
    change_stage(state, store, DeliveryStage::Verifying)?;
    emit_delivery(context, state)?;
    let receipt = reconcile(context, store, state, session_id, &summary.agent.agent_id).await;
    if let Err(receipt_error) = receipt {
        let error = if let Err(mut host_error) = sent {
            host_error
                .details
                .insert("receiptError".into(), receipt_error.code);
            host_error
        } else {
            receipt_error
        };
        mark_failure(state, store, error.clone())?;
        emit_delivery(context, state)?;
        return Err(error);
    }
    Ok(())
}

/// 上一次没交完（主人选了重试，或者中途断了）：先看上次的回复是不是其实已经发出去了。
/// 发出去了就把上一批收尾，返回 true；没有就照常交这一批，沿用原来的提交 ID 和回复目标。
async fn recover_previous(
    context: &ReceiverContext<'_>,
    store: &ReceiverStore,
    state: &mut ReceiverState,
    summary: &IpcSelfSummary,
    session_id: &str,
) -> Result<bool> {
    let Some(previous) = state
        .last_delivery
        .clone()
        .filter(|record| record.stage == DeliveryStage::Received)
    else {
        return Ok(false);
    };
    let after = state.checkpoint.cursor().map(str::to_owned);
    match crate::receipt::verify(
        context.backend,
        session_id,
        &state.binding,
        &previous,
        &summary.agent.agent_id,
        Duration::from_secs(2),
    )
    .await
    {
        Ok(_) => {
            state.checkpoint = ReceptionCheckpoint::Pending {
                after_event_id: after,
                event_id: previous.event_id,
            };
            reconcile(context, store, state, session_id, &summary.agent.agent_id).await?;
            Ok(true)
        }
        Err(error) if error.code == "receiver.reply_unconfirmed" => Ok(false),
        Err(error) => Err(error),
    }
}

/// 一批要交给宿主了：整批记成待定，锚在交出去的最后一条上。上次没交完的沿用原来的提交 ID
/// 和回复目标，哪怕上次其实发出去了也不会重复。
fn begin_batch(
    state: &mut ReceiverState,
    store: &ReceiverStore,
    anchor: &str,
    target: &str,
) -> Result<()> {
    state
        .checkpoint
        .begin(anchor)
        .map_err(|_| Failure::local("receiver.pending_review_required"))?;
    let previous = state
        .last_delivery
        .as_ref()
        .filter(|record| record.stage == DeliveryStage::Received);
    let (submission_id, message_id) = previous.map_or_else(
        || (uuid::Uuid::now_v7().to_string(), target.to_owned()),
        |record| (record.submission_id.clone(), record.message_id.clone()),
    );
    state.last_delivery = Some(DeliveryRecord {
        event_id: anchor.to_owned(),
        message_id,
        submission_id,
        stage: DeliveryStage::Received,
        reply_event_id: None,
        failure: None,
    });
    store.save(state)
}

async fn send_reply(
    context: &ReceiverContext<'_>,
    state: &ReceiverState,
    session_id: &str,
    reply: HostReply,
    record: &DeliveryRecord,
) -> Result<()> {
    match call(
        context.backend,
        scoped(session_id, reply.into_request(state, record)?),
    )
    .await
    {
        Ok(IpcResponse::SentMessage { message })
            if message.submission_id == record.submission_id =>
        {
            Ok(())
        }
        Ok(_) => Err(Failure::local("receiver.response_invalid")),
        Err(error) => Err(error),
    }
}

/// 宿主看过、觉得不用回：不发消息，这一批算处理完了。
async fn finish_silently(
    context: &ReceiverContext<'_>,
    store: &ReceiverStore,
    state: &mut ReceiverState,
    session_id: &str,
    anchor: &str,
) -> Result<()> {
    state
        .checkpoint
        .complete(anchor)
        .map_err(|_| Failure::local("receiver.checkpoint_mismatch"))?;
    acknowledge_in_bridge(context.backend, session_id, anchor).await;
    change_stage(state, store, DeliveryStage::NoReply)?;
    crate::execution::save(context.backend, Some(session_id), store, state).await?;
    emit_delivery(context, state)
}

async fn reconcile(
    context: &ReceiverContext<'_>,
    store: &ReceiverStore,
    state: &mut ReceiverState,
    session_id: &str,
    agent_id: &str,
) -> Result<()> {
    let record = state
        .last_delivery
        .as_ref()
        .ok_or_else(|| Failure::local("receiver.legacy_pending_review_required"))?;
    let result = crate::receipt::verify(
        context.backend,
        session_id,
        &state.binding,
        record,
        agent_id,
        Duration::from_secs(20),
    )
    .await;
    match result {
        Ok(event_id) => {
            state
                .checkpoint
                .complete(&record.event_id)
                .map_err(|_| Failure::local("receiver.checkpoint_mismatch"))?;
            acknowledge_in_bridge(context.backend, session_id, &record.event_id).await;
            let record = state
                .last_delivery
                .as_mut()
                .ok_or_else(|| Failure::local("receiver.delivery_missing"))?;
            record.stage = DeliveryStage::Replied;
            record.reply_event_id = Some(event_id);
            record.failure = None;
            store.save(state)?;
            crate::execution::save(context.backend, Some(session_id), store, state).await?;
            emit_delivery(context, state)
        }
        Err(mut error) => {
            // Retryable network errors may reconnect and verify, but must never rerun the host.
            if !error.retryable {
                error.code = "receiver.reply_unconfirmed".into();
            }
            mark_failure(state, store, error.clone())?;
            emit_delivery(context, state)?;
            Err(error)
        }
    }
}

/// 宿主处理完的这一批在 Bridge 上也算确认了：同一个人物之后用 MCP 或命令行不带位置等消息，
/// 不会再收到它们。确认失败不要紧，只是之后多看到几条已经处理过的。
async fn acknowledge_in_bridge(backend: &dyn BridgeToolClient, session_id: &str, anchor: &str) {
    let method = scoped(
        session_id,
        IpcMethod::AckInbox(IpcAckInboxRequest {
            id: anchor.to_owned(),
        }),
    );
    let _ = call(backend, method).await;
}

fn change_stage(
    state: &mut ReceiverState,
    store: &ReceiverStore,
    next: DeliveryStage,
) -> Result<()> {
    let record = state
        .last_delivery
        .as_mut()
        .ok_or_else(|| Failure::local("receiver.delivery_missing"))?;
    record.stage = next;
    store.save(state)
}
fn mark_failure(state: &mut ReceiverState, store: &ReceiverStore, error: Failure) -> Result<()> {
    let record = state
        .last_delivery
        .as_mut()
        .ok_or_else(|| Failure::local("receiver.delivery_missing"))?;
    record.stage = DeliveryStage::NeedsReview;
    record.failure = Some(error);
    store.save(state)
}
fn emit_delivery(context: &ReceiverContext<'_>, state: &ReceiverState) -> Result<()> {
    (context.emit)(ReceiverEvent::Delivery {
        record: state
            .last_delivery
            .clone()
            .ok_or_else(|| Failure::local("receiver.delivery_missing"))?,
    })
}

async fn wait_until_ready(
    backend: &dyn BridgeToolClient,
    session_id: &str,
) -> Result<IpcSelfSummary> {
    tokio::time::timeout(Duration::from_mins(2), async {
        loop {
            match call(backend, scoped(session_id, IpcMethod::GetSelf)).await {
                Ok(IpcResponse::SelfSummary { summary })
                    if summary.connection_state == IpcBridgeState::Ready =>
                {
                    return Ok(summary);
                }
                Ok(IpcResponse::SelfSummary { .. }) => {}
                Ok(_) => return Err(Failure::local("receiver.response_invalid")),
                Err(error) if error.retryable => {}
                Err(error) => return Err(error),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
    .await
    // A prolonged outage must return to reconnect backoff, not stop an enabled receiver.
    .map_err(|_| Failure {
        category: agent_room_bridge_ipc::IpcErrorCategory::DependencyUnavailable,
        retryable: true,
        ..Failure::local("receiver.session_start_timeout")
    })?
}

async fn initial_cursor(
    backend: &dyn BridgeToolClient,
    binding: &ReceiverBinding,
    session_id: &str,
) -> Result<Option<String>> {
    match &binding.start {
        ReceiverStart::After { event_id } => Ok(Some(event_id.clone())),
        ReceiverStart::Beginning => Ok(None),
        ReceiverStart::Now => {
            let mut request = binding.inbox_request(None);
            request.limit = 1;
            let IpcResponse::MessagePreviews { previews, .. } = call(
                backend,
                scoped(session_id, IpcMethod::ListPreviews(request)),
            )
            .await?
            else {
                return Err(Failure::local("receiver.response_invalid"));
            };
            Ok(previews.first().map(|message| message.event_id.clone()))
        }
    }
}
