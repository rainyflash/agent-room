use crate::{
    DeliveryRecord, DeliveryStage, ReceiverExecution, ReceiverState, ReceiverStore,
    ReceptionFailure as Failure, ReceptionResult as Result, call, scoped,
};
use agent_room_agent_client::{BridgeToolClient, reception::ReceptionCheckpoint};
use agent_room_bridge_ipc::{
    IpcMethod, IpcResponse, ReceptionCommand, ReceptionPending, ReceptionProgress, ReceptionRecord,
    ReceptionRequest, ReceptionStatus,
};
use uuid::Uuid;

fn id(value: Option<&str>) -> Result<Uuid> {
    value
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| Failure::local("reception.identity_unavailable"))
}
fn pending(record: &DeliveryRecord) -> Result<ReceptionPending> {
    Ok(ReceptionPending {
        event_id: record.event_id.clone(),
        message_id: id(Some(&record.message_id))?,
        submission_id: id(Some(&record.submission_id))?,
    })
}
pub(crate) fn progress(state: &ReceiverState) -> Result<ReceptionProgress> {
    let mut progress = ReceptionProgress {
        after_event_id: state.checkpoint.cursor().map(str::to_owned),
        ..ReceptionProgress::default()
    };
    if matches!(state.checkpoint, ReceptionCheckpoint::Pending { .. }) {
        progress.pending =
            Some(pending(state.last_delivery.as_ref().ok_or_else(|| {
                Failure::local("receiver.legacy_pending_review_required")
            })?)?);
    } else if let Some(record) = &state.last_delivery
        && record.stage == DeliveryStage::Received
    {
        progress.retry = Some(pending(record)?);
    }
    Ok(progress)
}
pub(crate) fn request(
    state: &ReceiverState,
    command: ReceptionCommand,
) -> Result<ReceptionRequest> {
    Ok(ReceptionRequest {
        agent_id: id(state.agent_id.as_deref())?,
        instance_id: id(state.instance_id.as_deref())?,
        catalog_id: id(state.room_catalog_id.as_deref())?,
        room_id: state.binding.policy.room_id.clone(),
        run_id: state
            .execution
            .as_ref()
            .ok_or_else(|| Failure::local("reception.execution_missing"))?
            .run_id,
        command,
    })
}
async fn control(
    backend: &dyn BridgeToolClient,
    session: Option<&str>,
    request: ReceptionRequest,
) -> Result<ReceptionRecord> {
    let method = IpcMethod::ReceptionControl(request);
    let response = call(
        backend,
        session.map_or_else(|| method.clone(), |session| scoped(session, method.clone())),
    )
    .await?;
    if let IpcResponse::Reception { record } = response {
        Ok(record)
    } else {
        Err(Failure::local("reception.invalid_response"))
    }
}
pub(crate) async fn claim(
    backend: &dyn BridgeToolClient,
    session: &str,
    store: &ReceiverStore,
    state: &mut ReceiverState,
) -> Result<()> {
    if state.execution.is_none() {
        state.execution = Some(ReceiverExecution {
            run_id: Uuid::now_v7(),
            revision: 0,
            releasing: false,
        });
        store.save(state)?;
    }
    let local = progress(state)?;
    let command = ReceptionCommand::Claim {
        session_key: id(Some(&state.binding.session.session_key))?,
        display_name: state.binding.session.display_name.clone(),
        initial: local.clone(),
    };
    let record = match control(backend, Some(session), request(state, command)?).await {
        Ok(record) => record,
        Err(error) => {
            if matches!(
                error.code.as_str(),
                "reception.execution_conflict" | "reception.forbidden"
            ) {
                state.execution = None;
                store.save(state)?;
            }
            return Err(error);
        }
    };
    if record.status != ReceptionStatus::Active {
        return Err(Failure::local("reception.execution_conflict"));
    }
    state
        .execution
        .as_mut()
        .ok_or_else(|| Failure::local("reception.execution_missing"))?
        .revision = record.revision;
    let explicit_resolution = record.progress.pending.as_ref().is_some_and(|pending| {
        state.last_delivery.as_ref().is_some_and(|delivery| {
            delivery.event_id == pending.event_id
                && matches!(
                    delivery.stage,
                    DeliveryStage::Received | DeliveryStage::Skipped
                )
        }) && matches!(state.checkpoint, ReceptionCheckpoint::Ready { .. })
    });
    if !explicit_resolution {
        restore(state, &record.progress);
    }
    store.save(state)?;
    if explicit_resolution {
        save(backend, Some(session), store, state).await?;
    }
    Ok(())
}
fn restore(state: &mut ReceiverState, progress: &ReceptionProgress) {
    state.checkpoint = match &progress.pending {
        Some(pending) => ReceptionCheckpoint::Pending {
            after_event_id: progress.after_event_id.clone(),
            event_id: pending.event_id.clone(),
        },
        None => ReceptionCheckpoint::Ready {
            after_event_id: progress.after_event_id.clone(),
        },
    };
    if let Some(record) = progress.pending.as_ref().or(progress.retry.as_ref()) {
        state.last_delivery = Some(DeliveryRecord {
            event_id: record.event_id.clone(),
            message_id: record.message_id.to_string(),
            submission_id: record.submission_id.to_string(),
            stage: if progress.pending.is_some() {
                DeliveryStage::NeedsReview
            } else {
                DeliveryStage::Received
            },
            reply_event_id: None,
            failure: None,
        });
    } else if state.last_delivery.as_ref().is_some_and(|record| {
        !matches!(
            record.stage,
            DeliveryStage::Replied | DeliveryStage::Skipped
        )
    }) {
        state.last_delivery = None;
    }
}
pub(crate) async fn save(
    backend: &dyn BridgeToolClient,
    session: Option<&str>,
    store: &ReceiverStore,
    state: &mut ReceiverState,
) -> Result<()> {
    let revision = state
        .execution
        .as_ref()
        .ok_or_else(|| Failure::local("reception.execution_missing"))?
        .revision;
    let record = control(
        backend,
        session,
        request(
            state,
            ReceptionCommand::Save {
                revision,
                progress: progress(state)?,
            },
        )?,
    )
    .await?;
    state
        .execution
        .as_mut()
        .ok_or_else(|| Failure::local("reception.execution_missing"))?
        .revision = record.revision;
    store.save(state)
}
/// Must only be called after the old host session and its in-flight calls drain.
pub(crate) async fn release(
    backend: &dyn BridgeToolClient,
    store: &ReceiverStore,
    state: &mut ReceiverState,
) -> Result<()> {
    let Some(execution) = &mut state.execution else {
        return Ok(());
    };
    execution.releasing = true;
    store.save(state)?;
    // A stopped session cannot issue new calls; the root transport carries the same
    // signed device proof, scoped again by the server to this exact owner and run.
    let record = control(backend, None, request(state, ReceptionCommand::Release)?).await?;
    if record.status != ReceptionStatus::Idle {
        return Err(Failure::local("reception.stop_unconfirmed"));
    }
    state.execution = None;
    store.save(state)
}
pub(crate) async fn monitor(
    backend: &dyn BridgeToolClient,
    session: &str,
    request: ReceptionRequest,
) -> Result<()> {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let record = control(backend, Some(session), request.clone()).await?;
        if record.status != ReceptionStatus::Active {
            return Err(Failure::local("reception.handoff_requested"));
        }
    }
}
