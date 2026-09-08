use crate::{BridgeToolClient, BridgeToolFailure};
use agent_room_bridge_ipc::{IpcErrorCategory, IpcListPreviewsRequest, IpcMethod, IpcResponse};
use std::{collections::BTreeMap, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageReadMode {
    History,
    Inbox,
}

/// Wait for a bounded message page. Dropping the future cancels the wait and leaves the cursor
/// unchanged. The consumer advances its cursor only after processing a returned page.
///
/// # Errors
/// Returns validation, transport or Bridge failures without pretending that the inbox is empty.
pub async fn wait_for_messages(
    backend: &dyn BridgeToolClient,
    session_id: String,
    request: IpcListPreviewsRequest,
    mode: MessageReadMode,
    wait_seconds: u8,
) -> Result<IpcResponse, BridgeToolFailure> {
    if wait_seconds > 25 || (wait_seconds > 0 && request.before_event_id.is_some()) {
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
    if wait_seconds == 0 {
        return backend.invoke(method).await;
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(u64::from(wait_seconds));
    let mut last_empty = None;
    loop {
        let response = match tokio::time::timeout_at(deadline, backend.invoke(method.clone())).await
        {
            Ok(response) => response?,
            Err(_) => {
                return last_empty.ok_or_else(|| {
                    failure(
                        "agent.inbox.timeout",
                        IpcErrorCategory::DependencyUnavailable,
                        true,
                    )
                });
            }
        };
        if !matches!(&response, IpcResponse::MessagePreviews { previews, .. } if previews.is_empty())
        {
            return Ok(response);
        }
        last_empty = Some(response);
        tokio::time::sleep_until(std::cmp::min(
            deadline,
            tokio::time::Instant::now() + Duration::from_secs(1),
        ))
        .await;
        if tokio::time::Instant::now() >= deadline {
            return last_empty.ok_or_else(|| {
                failure(
                    "agent.inbox.response_missing",
                    IpcErrorCategory::Internal,
                    false,
                )
            });
        }
    }
}

fn failure(code: &str, category: IpcErrorCategory, retryable: bool) -> BridgeToolFailure {
    BridgeToolFailure::new(code, category, retryable, BTreeMap::new())
}
