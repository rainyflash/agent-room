use crate::{
    DeliveryRecord, DeliveryStage, ReceiverBinding, ReceiverEvent, ReceiverStart, ReceiverState,
    ReceiverStore, ReceptionFailure as Failure, ReceptionResult as Result, call, scoped,
};
use agent_room_agent_client::{
    BridgeToolClient, MessageReadMode,
    reception::{DeliveryDecision, ReceptionCheckpoint},
    wait_for_messages,
};
use agent_room_bridge_ipc::{
    IpcBridgeState, IpcCloseHostSessionRequest, IpcHostSessionState, IpcMessagePreviewSummary,
    IpcMethod, IpcResponse, IpcSelfSummary,
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
    state.binding.host.validate()?;
    let mut session_id = None;
    let result = tokio::select! {
        biased;
        () = stop => Ok(()),
        result = receive_loop(&context, &store, &mut state, &mut session_id) => result,
    };
    let close = if let Some(session_id) = session_id {
        call(
            context.backend,
            IpcMethod::CloseHostSession(IpcCloseHostSessionRequest { session_id }),
        )
        .await
        .map(|_| ())
    } else {
        Ok(())
    };
    if let Err(mut error) = result {
        if let Err(close_error) = close {
            error.details.insert("closeError".into(), close_error.code);
        }
        return Err(error);
    }
    close?;
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
            Err(error) if error.retryable => {
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
        store.save(state)?;
    }
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
    poll_inbox(context, store, state, &summary, &session.session_id).await
}

async fn poll_inbox(
    context: &ReceiverContext<'_>,
    store: &ReceiverStore,
    state: &mut ReceiverState,
    summary: &IpcSelfSummary,
    session_id: &str,
) -> Result<()> {
    let binding = state.binding.clone();
    loop {
        let response = wait_for_messages(
            context.backend,
            session_id.to_owned(),
            binding.inbox_request(state.checkpoint.cursor().map(str::to_owned)),
            MessageReadMode::Inbox,
            25,
        )
        .await
        .map_err(|error| {
            let mut failure = Failure::from(error);
            if matches!(
                failure.code.as_str(),
                "bridge.host_session.not_found" | "bridge.host_session.closed"
            ) {
                failure.retryable = true;
            }
            failure
        })?;
        let IpcResponse::MessagePreviews { previews, .. } = response else {
            return Err(Failure::local("receiver.response_invalid"));
        };
        for message in previews {
            tokio::task::yield_now().await;
            deliver(context, store, state, summary, session_id, &message).await?;
        }
    }
}

async fn deliver(
    context: &ReceiverContext<'_>,
    store: &ReceiverStore,
    state: &mut ReceiverState,
    summary: &IpcSelfSummary,
    session_id: &str,
    message: &IpcMessagePreviewSummary,
) -> Result<()> {
    let binding = state.binding.clone();
    let retry = state
        .last_delivery
        .as_ref()
        .is_some_and(|record| record.event_id == message.event_id);
    if !prepare_delivery(state, store, message, &summary.agent.matrix_user_id)? {
        return Ok(());
    }
    emit_delivery(context, state)?;
    let record = state
        .last_delivery
        .clone()
        .ok_or_else(|| Failure::local("receiver.delivery_missing"))?;
    if retry {
        match crate::receipt::verify(
            context.backend,
            session_id,
            &binding,
            &record,
            &summary.agent.agent_id,
            Duration::from_secs(2),
        )
        .await
        {
            Ok(_) => {
                reconcile(context, store, state, session_id, &summary.agent.agent_id).await?;
                return Ok(());
            }
            Err(error) if error.code == "receiver.reply_unconfirmed" => {}
            Err(error) => return Err(error),
        }
    }
    change_stage(state, store, DeliveryStage::Running)?;
    emit_delivery(context, state)?;
    let host = context
        .host
        .resume(crate::HostDelivery {
            binding: &binding.host,
            data_root: context.data_root,
            service: context.service,
            session_id,
            automation_grant_id: &binding.automation_grant_id,
            submission_id: &record.submission_id,
            message,
        })
        .await;
    change_stage(state, store, DeliveryStage::Verifying)?;
    emit_delivery(context, state)?;
    let receipt = reconcile(context, store, state, session_id, &summary.agent.agent_id).await;
    if let Err(receipt_error) = receipt {
        let error = if let Err(mut host_error) = host {
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

pub(crate) fn prepare_delivery(
    state: &mut ReceiverState,
    store: &ReceiverStore,
    message: &IpcMessagePreviewSummary,
    matrix_user_id: &str,
) -> Result<bool> {
    let decision = state
        .checkpoint
        .prepare(message, &state.binding.policy, matrix_user_id)
        .map_err(|_| Failure::local("receiver.pending_review_required"))?;
    if decision == DeliveryDecision::Skip {
        store.save(state)?;
        return Ok(false);
    }
    let previous = state
        .last_delivery
        .as_ref()
        .filter(|record| record.event_id == message.event_id);
    // The identity is stable even if the owner explicitly retries after a failed turn.
    let submission_id = previous.map_or_else(
        || uuid::Uuid::now_v7().to_string(),
        |record| record.submission_id.clone(),
    );
    state.last_delivery = Some(DeliveryRecord {
        event_id: message.event_id.clone(),
        message_id: message.message_id.clone(),
        submission_id,
        stage: DeliveryStage::Received,
        reply_event_id: None,
        failure: None,
    });
    store.save(state)?;
    Ok(true)
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
            let record = state
                .last_delivery
                .as_mut()
                .ok_or_else(|| Failure::local("receiver.delivery_missing"))?;
            record.stage = DeliveryStage::Replied;
            record.reply_event_id = Some(event_id);
            record.failure = None;
            store.save(state)?;
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
    .map_err(|_| Failure::local("receiver.session_start_timeout"))?
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
