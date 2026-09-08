mod cli;
mod output;
mod receiver;

use agent_room_agent_client::{
    BridgeToolClient, LocalBridgeToolClient, MessageReadMode, wait_for_messages,
};
use agent_room_bridge_ipc::{
    IpcCloseHostSessionRequest, IpcListPreviewsRequest, IpcMethod, IpcOpenHostSessionRequest,
    IpcPublishStatusRequest, IpcResponse, IpcWorkStatus,
};
use agent_room_bridge_local_adapter::{
    bridge_data_root_from_environment, bridge_runtime_root, secure_storage_service_from_environment,
};
use clap::Parser;
use cli::{Cli, Command, SessionCommand, WorkStatus};
use output::{CliFailure, CliResult, success, write_json};
use serde_json::json;
use std::{io::Read as _, process::ExitCode};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if !error.use_stderr() => {
            return if error.print().is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
        Err(error) => {
            let mut failure = CliFailure::validation("cli.arguments_invalid");
            failure.details.insert("help".into(), error.to_string());
            return report_failure(&failure);
        }
    };
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_failure(&error),
    }
}

fn report_failure(error: &CliFailure) -> ExitCode {
    if write_json(&json!({"ok": false, "error": error})).is_err() {
        eprintln!("cli.output_failed");
    }
    ExitCode::from(error.exit_code())
}

async fn run(cli: Cli) -> CliResult<()> {
    let data_root = match cli.data_root {
        Some(path) if path.is_absolute() => path,
        Some(_) => return Err(CliFailure::validation("cli.data_root_must_be_absolute")),
        None => bridge_data_root_from_environment()
            .map_err(|_| CliFailure::validation("cli.data_root_invalid"))?,
    };
    let service = secure_storage_service_from_environment()
        .map_err(|_| CliFailure::validation("cli.secure_storage_service_invalid"))?;
    let backend =
        LocalBridgeToolClient::agent_cli(bridge_runtime_root(&data_root), service.clone());
    match cli.command {
        Command::Doctor => success(call(&backend, IpcMethod::BridgeStatus).await?),
        Command::Session {
            action: SessionCommand::Open { name, key },
        } => {
            let key = key.unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
            let result = call(
                &backend,
                IpcMethod::OpenHostSession(IpcOpenHostSessionRequest {
                    session_key: key.clone(),
                    display_name: name,
                }),
            )
            .await?;
            success(json!({"sessionKey": key, "result": result}))
        }
        Command::Session {
            action: SessionCommand::Close(args),
        } => success(
            call(
                &backend,
                IpcMethod::CloseHostSession(IpcCloseHostSessionRequest {
                    session_id: args.session,
                }),
            )
            .await?,
        ),
        Command::Whoami(args) => {
            success(call(&backend, scoped(args.session, IpcMethod::GetSelf)).await?)
        }
        Command::Read(args) => success(read(&backend, &args).await?),
        Command::Listen(args) => listen(&backend, args).await,
        Command::Send(args) => send(&backend, args).await,
        Command::Status(args) => {
            let status = match args.value {
                WorkStatus::Offline => IpcWorkStatus::Offline,
                WorkStatus::Idle => IpcWorkStatus::Idle,
                WorkStatus::Working => IpcWorkStatus::Working,
                WorkStatus::WaitingInput => IpcWorkStatus::WaitingInput,
                WorkStatus::Blocked => IpcWorkStatus::Blocked,
                WorkStatus::Completed => IpcWorkStatus::Completed,
            };
            success(
                call(
                    &backend,
                    scoped(
                        args.session,
                        IpcMethod::PublishStatus(IpcPublishStatusRequest {
                            room_id: args.room,
                            status,
                            task_summary: args.summary,
                            progress_basis_points: None,
                        }),
                    ),
                )
                .await?,
            )
        }
        Command::Receive { binding } => {
            receiver::run(
                &backend,
                &data_root,
                service.as_str(),
                &binding,
                agent_room_agent_reception::ReceiverMode::Listen,
            )
            .await
        }
        Command::Receiver {
            action: cli::ReceiverCommand::Verify { binding },
        } => {
            receiver::run(
                &backend,
                &data_root,
                service.as_str(),
                &binding,
                agent_room_agent_reception::ReceiverMode::VerifyReceipt,
            )
            .await
        }
        Command::Receiver { action } => receiver::manage(&data_root, action),
    }
}

async fn read(backend: &dyn BridgeToolClient, args: &cli::ReadArgs) -> CliResult<IpcResponse> {
    Ok(wait_for_messages(
        backend,
        args.session.clone(),
        IpcListPreviewsRequest {
            room_id: args.room.clone(),
            after_event_id: args.after.clone(),
            before_event_id: None,
            limit: args.limit,
        },
        MessageReadMode::Inbox,
        args.wait,
    )
    .await?)
}

pub(crate) async fn call(
    backend: &dyn BridgeToolClient,
    method: IpcMethod,
) -> CliResult<IpcResponse> {
    method
        .validate()
        .map_err(|error| CliFailure::validation(error.code()))?;
    Ok(backend.invoke(method).await?)
}
pub(crate) fn scoped(session_id: String, method: IpcMethod) -> IpcMethod {
    IpcMethod::WithSession {
        session_id,
        method: Box::new(method),
    }
}

fn chat_request(
    session: &str,
    room: &str,
    body: String,
    submission_id: String,
    reply_to: Option<String>,
    mentions: Vec<String>,
    automation_grant_id: Option<String>,
) -> IpcMethod {
    use agent_room_bridge_ipc::{
        IpcMessageProvenance, IpcMessageSensitivity, IpcSendMessageRequest,
    };
    let provenance = if automation_grant_id.is_some() {
        IpcMessageProvenance::AutonomousAgent
    } else {
        IpcMessageProvenance::HumanConfirmedAgent
    };
    scoped(
        session.into(),
        IpcMethod::SendMessage(IpcSendMessageRequest {
            chat: true,
            mentions,
            submission_id: Some(submission_id),
            automation_grant_id,
            room_id: room.into(),
            title: body.chars().take(120).collect(),
            summary: body.chars().take(280).collect(),
            body,
            media_type: "text/plain".into(),
            language: None,
            sensitivity: IpcMessageSensitivity::Normal,
            risk_flags: vec![],
            provenance,
            reply_to_message_id: reply_to,
        }),
    )
}

async fn listen(backend: &dyn BridgeToolClient, mut args: cli::ReadArgs) -> CliResult<()> {
    if args.wait == 0 {
        return Err(CliFailure::validation("cli.listen_wait_must_be_positive"));
    }
    loop {
        let response = tokio::select! { result = read(backend, &args) => result?, signal = tokio::signal::ctrl_c() => {
            signal.map_err(|_| CliFailure::local("cli.signal_failed"))?; return success(json!({"type": "stopped", "afterEventId": args.after}));
        }};
        let IpcResponse::MessagePreviews { previews, .. } = &response else {
            return Err(CliFailure::local("cli.response_invalid"));
        };
        if let Some(last) = previews.last() {
            args.after = Some(last.event_id.clone());
        }
        success(response)?;
    }
}

async fn send(backend: &dyn BridgeToolClient, args: cli::SendArgs) -> CliResult<()> {
    let body = if args.stdin {
        let mut body = String::new();
        std::io::stdin()
            .take(64 * 1024 + 1)
            .read_to_string(&mut body)
            .map_err(|_| CliFailure::validation("cli.stdin_invalid"))?;
        if body.len() > 64 * 1024 {
            return Err(CliFailure::validation("cli.message_too_large"));
        }
        body
    } else {
        args.text
            .ok_or_else(|| CliFailure::validation("cli.text_required"))?
    };
    let request = chat_request(
        &args.session,
        &args.room,
        body,
        args.submission_id,
        args.reply_to,
        args.mention,
        args.automation_grant,
    );
    success(call(backend, request).await?)
}
