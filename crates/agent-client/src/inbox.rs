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
    /// A continuous listener's refresh window. Reaching it ends one round and the caller keeps
    /// listening, so the window never cancels a call that is already in flight.
    Continuous(Duration),
}

impl MessageWait {
    #[must_use]
    pub fn from_seconds(seconds: Option<u32>) -> Self {
        seconds.map_or(Self::UntilMessage, |seconds| {
            Self::For(Duration::from_secs(u64::from(seconds)))
        })
    }

    /// The same explicit window, for a caller that keeps listening after it elapses.
    #[must_use]
    pub fn continuous_from_seconds(seconds: Option<u32>) -> Self {
        seconds.map_or(Self::UntilMessage, |seconds| {
            Self::Continuous(Duration::from_secs(u64::from(seconds)))
        })
    }
}

/// Wait inside the tool, without returning empty pages to the model unless the caller requests
/// a deadline or a listening window. Dropping the future cancels the wait and leaves the cursor
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
        session_id: session_id.clone(),
        method: Box::new(match mode {
            MessageReadMode::History => IpcMethod::ListPreviews(request.clone()),
            MessageReadMode::Inbox if !immediate => IpcMethod::WaitInbox(request.clone()),
            MessageReadMode::Inbox => IpcMethod::ReadInbox(request.clone()),
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
        MessageWait::For(duration) | MessageWait::Continuous(duration) => {
            Some(tokio::time::Instant::now() + duration)
        }
    };
    // Only a caller that stops at its deadline may cut off a call in flight: it owes an answer by
    // then and a stalled Bridge must not pass for an empty room. A listener owes no answer, so its
    // window only ends the round. Cutting the call off there would turn one slow local round trip
    // into the end of the stream, while the single IPC still has its own connect and operation
    // deadlines to report a Bridge that never answers.
    let answer_due = matches!(wait, MessageWait::For(_));
    loop {
        let response = if let Some(deadline) = deadline.filter(|_| answer_due) {
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
            return if mode == MessageReadMode::Inbox {
                // Finish the wait explicitly. A killed process is covered by the short wait lease.
                backend
                    .invoke(IpcMethod::WithSession {
                        session_id,
                        method: Box::new(IpcMethod::ReadInbox(request)),
                    })
                    .await
            } else {
                Ok(response)
            };
        }
    }
}

fn failure(code: &str, category: IpcErrorCategory, retryable: bool) -> BridgeToolFailure {
    BridgeToolFailure::new(code, category, retryable, BTreeMap::new())
}
