use crate::{
    cli::{ReceiverCommand, Resolution},
    output::{CliFailure, CliResult, success},
};
use agent_room_agent_client::BridgeToolClient;
use agent_room_agent_reception::{ReceiverContext, ReceiverStore, load_binding};
use std::path::Path;

pub(crate) async fn run(
    backend: &dyn BridgeToolClient,
    data_root: &Path,
    service: &str,
    path: &Path,
    mode: agent_room_agent_reception::ReceiverMode,
) -> CliResult<()> {
    let binding = load_binding(path)?;
    let task_id = binding.host.task_id.clone();
    if mode == agent_room_agent_reception::ReceiverMode::Listen {
        let store = ReceiverStore::open(data_root, &task_id)?;
        store.configure(binding, service)?;
    }
    let emit = |event| {
        success(event)
            .map_err(|error| agent_room_agent_reception::ReceptionFailure::local(&error.code))
    };
    let context = ReceiverContext {
        host: &agent_room_agent_reception::NativeHost,
        mode,
        backend,
        data_root,
        service,
        emit: &emit,
    };
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let signal = tokio::spawn(async move {
        let result = tokio::signal::ctrl_c().await;
        let _ = stop.send(());
        result
    });
    let result = agent_room_agent_reception::run(context, &task_id, async {
        let _ = stopped.await;
    })
    .await;
    if signal.is_finished() {
        signal
            .await
            .map_err(|_| CliFailure::local("cli.signal_failed"))?
            .map_err(|_| CliFailure::local("cli.signal_failed"))?;
    } else {
        signal.abort();
    }
    result.map_err(Into::into)
}

pub(crate) fn manage(data_root: &Path, action: ReceiverCommand) -> CliResult<()> {
    let path = match &action {
        ReceiverCommand::Inspect { binding }
        | ReceiverCommand::Resolve { binding, .. }
        | ReceiverCommand::Update { binding } => binding,
        ReceiverCommand::List => return success(ReceiverStore::list(data_root)?),
        ReceiverCommand::Verify { .. } => {
            return Err(CliFailure::local("receiver.verification_requires_runtime"));
        }
    };
    let binding = load_binding(path)?;
    if matches!(&action, ReceiverCommand::Inspect { .. }) {
        return success(ReceiverStore::inspect(data_root, &binding.host.task_id)?);
    }
    let store = ReceiverStore::open(data_root, &binding.host.task_id)?;
    match action {
        ReceiverCommand::Resolve { event, action, .. } => success(store.resolve(
            &event,
            match action {
                Resolution::Retry => agent_room_agent_reception::Resolution::Retry,
                Resolution::Skip => agent_room_agent_reception::Resolution::Skip,
            },
        )?),
        ReceiverCommand::Update { .. } => {
            let state = store
                .load()?
                .ok_or_else(|| CliFailure::local("receiver.state_missing"))?;
            success(store.configure(binding, &state.bridge_service)?)
        }
        ReceiverCommand::Inspect { .. }
        | ReceiverCommand::List
        | ReceiverCommand::Verify { .. } => unreachable!(),
    }
}
