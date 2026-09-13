use crate::{BridgeToolClient, BridgeToolFailure};
use agent_room_bridge_ipc::{IpcErrorCategory, IpcListPreviewsRequest, IpcMethod, IpcResponse};
use std::{collections::BTreeMap, time::Duration};

pub const MAX_EXPLICIT_WAIT_SECONDS: u32 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageReadMode {
    History,
    Inbox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageWait {
    UntilMessage,
    /// An explicit caller deadline; zero performs one immediate read.
    For(Duration),
}

impl MessageWait {
    #[must_use]
    pub fn from_seconds(seconds: Option<u32>) -> Self {
        seconds.map_or(Self::UntilMessage, |seconds| {
            Self::For(Duration::from_secs(u64::from(seconds)))
        })
    }
}

/// Wait inside the tool, without returning empty pages to the model unless the caller requests
/// a deadline. Dropping the future cancels the wait and leaves the cursor
/// unchanged. The consumer advances its cursor only after processing a returned page.
///
/// # Errors
/// Returns validation, transport or Bridge failures without pretending that the inbox is empty.
pub async fn wait_for_messages(
    backend: &dyn BridgeToolClient,
    session_id: String,
    request: IpcListPreviewsRequest,
    mode: MessageReadMode,
    wait: MessageWait,
) -> Result<IpcResponse, BridgeToolFailure> {
    let immediate = wait == MessageWait::For(Duration::ZERO);
    if !immediate && request.before_event_id.is_some() {
        return Err(failure(
            "agent.inbox.wait_invalid",
            IpcErrorCategory::Validation,
            false,
        ));
    }
    let method = IpcMethod::WithSession {
        session_id,
        method: Box::new(match mode {
            MessageReadMode::History => IpcMethod::ListPreviews(request),
            MessageReadMode::Inbox => IpcMethod::ReadInbox(request),
        }),
    };
    method
        .validate()
        .map_err(|error| failure(error.code(), IpcErrorCategory::Validation, false))?;
    if immediate {
        return backend.invoke(method).await;
    }
    let deadline = match wait {
        MessageWait::UntilMessage => None,
        MessageWait::For(duration) => Some(tokio::time::Instant::now() + duration),
    };
    loop {
        let response = if let Some(deadline) = deadline {
            tokio::time::timeout_at(deadline, backend.invoke(method.clone()))
                .await
                .map_err(|_| {
                    failure(
                        "agent.inbox.timeout",
                        IpcErrorCategory::DependencyUnavailable,
                        true,
                    )
                })??
        } else {
            backend.invoke(method.clone()).await?
        };
        if !matches!(&response, IpcResponse::MessagePreviews { previews, .. } if previews.is_empty())
        {
            return Ok(response);
        }
        // Bridge owns Matrix sync and the inbox. This local check never invokes a model.
        let next_check = tokio::time::Instant::now() + Duration::from_secs(1);
        tokio::time::sleep_until(deadline.map_or(next_check, |end| end.min(next_check))).await;
        if deadline.is_some_and(|end| tokio::time::Instant::now() >= end) {
            return Ok(response);
        }
    }
}

fn failure(code: &str, category: IpcErrorCategory, retryable: bool) -> BridgeToolFailure {
    BridgeToolFailure::new(code, category, retryable, BTreeMap::new())
}
