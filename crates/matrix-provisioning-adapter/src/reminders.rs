//! 提醒那一轮以应用服务替一个人读聊天服务器（`specs/redesign/reminders.md` 的“怎么算”）：他的
//! 个人工作区，和某一间房里某一刻以后的消息。请求带 `user_id`：应用服务的命名空间覆盖所有本地用户
//! （ADR 0011），读到的是他自己看得到的那一份，Synapse 照他的忽略名单过滤。
//!
//! 只取算提醒要用的：谁发的、什么时候、事件 ID，不加密的房间再看有没有点他的名字。正文不留。

use agent_room_application::ports::{
    MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId, MatrixUserId, PortFuture,
    ReminderMatrixReader, ReminderMessage, ReminderMessagePage,
};
use agent_room_domain::time::UtcMillis;
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    MatrixApplicationServiceProvisioner, decode_json, decode_matrix_error, map_matrix_error,
    map_transport_error, read_body_within, rooms::endpoint_with_segments,
};

const PERSONAL_WORKSPACE_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.personal-workspace.v1";
/// 个人工作区最多一万条，给足余量。
const MAX_WORKSPACE_RESPONSE_BYTES: usize = 8 * 1_024 * 1_024;
/// 加密的房间：服务器只看得到外面这一层，消息和修订分不开，都算。
const ENCRYPTED_EVENT_TYPES: [&str; 1] = ["m.room.encrypted"];
/// 不加密的房间：只看消息预览，修订不算新消息。v1 是 Agent 发的，v2 是网页上的人发的。
const PREVIEW_EVENT_TYPES: [&str; 2] = [
    "io.github.rainyflash.agentroom.message.preview.v1",
    "io.github.rainyflash.agentroom.message.preview.v2",
];
/// 每页这么多条，最多翻这么多页。
const PAGE_LIMIT: u16 = 100;
const MAX_PAGES: usize = 2;
/// 一页 100 条消息事件正常远小于这个数。
const MAX_PAGE_RESPONSE_BYTES: usize = 4 * 1_024 * 1_024;

/// `/messages` 的回答里只用得到这两样。
#[derive(Deserialize)]
struct MessagesPage {
    #[serde(default)]
    chunk: Vec<Value>,
    end: Option<String>,
}

impl MatrixApplicationServiceProvisioner {
    async fn personal_workspace_internal(
        &self,
        user: &MatrixUserId,
    ) -> MatrixResult<Option<Value>> {
        let operation = MatrixOperation::ReadAccountData;
        let url = endpoint_with_segments(
            &self.homeserver_url,
            &[
                "_matrix",
                "client",
                "v3",
                "user",
                user.as_str(),
                "account_data",
                PERSONAL_WORKSPACE_EVENT_TYPE,
            ],
            operation,
        )?;
        match self
            .read_as_user(url, user, operation, MAX_WORKSPACE_RESPONSE_BYTES)
            .await
        {
            Ok(body) => decode_json(&body, operation).map(Some),
            Err(failure) if failure.kind() == MatrixFailureKind::NotFound => Ok(None),
            Err(failure) => Err(failure),
        }
    }

    async fn messages_since_internal(
        &self,
        user: &MatrixUserId,
        room: &MatrixRoomId,
        end_to_end_encrypted: bool,
        since: UtcMillis,
    ) -> MatrixResult<ReminderMessagePage> {
        let operation = MatrixOperation::Backfill;
        let types: &[&str] = if end_to_end_encrypted {
            &ENCRYPTED_EVENT_TYPES
        } else {
            &PREVIEW_EVENT_TYPES
        };
        let filter = json!({ "types": types }).to_string();
        let mut messages = Vec::new();
        let mut from: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut url = endpoint_with_segments(
                &self.homeserver_url,
                &[
                    "_matrix",
                    "client",
                    "v3",
                    "rooms",
                    room.as_str(),
                    "messages",
                ],
                operation,
            )?;
            url.query_pairs_mut()
                .append_pair("dir", "b")
                .append_pair("limit", &PAGE_LIMIT.to_string())
                .append_pair("filter", &filter);
            if let Some(token) = &from {
                url.query_pairs_mut().append_pair("from", token);
            }
            let body = self
                .read_as_user(url, user, operation, MAX_PAGE_RESPONSE_BYTES)
                .await?;
            let page: MessagesPage = decode_json(&body, operation)?;
            // 从新往旧：翻到比问的那一刻早的就够了。
            for raw in &page.chunk {
                let Some(message) = reminder_message(raw, user, end_to_end_encrypted) else {
                    continue;
                };
                if message.sent_at < since {
                    return Ok(ReminderMessagePage {
                        messages,
                        truncated: false,
                    });
                }
                messages.push(message);
            }
            match page.end {
                Some(end) if !page.chunk.is_empty() => from = Some(end),
                _ => {
                    return Ok(ReminderMessagePage {
                        messages,
                        truncated: false,
                    });
                }
            }
        }
        Ok(ReminderMessagePage {
            messages,
            truncated: true,
        })
    }

    /// 以应用服务替 `user` 发一个 GET，成功时交回不超过 `limit` 字节的正文。
    async fn read_as_user(
        &self,
        mut url: Url,
        user: &MatrixUserId,
        operation: MatrixOperation,
        limit: usize,
    ) -> MatrixResult<Vec<u8>> {
        url.query_pairs_mut().append_pair("user_id", user.as_str());
        let response = self
            .client
            .get(url)
            .bearer_auth(self.access_token.expose())
            .send()
            .await
            .map_err(|error| map_transport_error(operation, &error))?;
        let status = response.status();
        let body = read_body_within(response, operation, limit).await?;
        if !status.is_success() {
            let error = decode_matrix_error(&body, operation)?;
            return Err(map_matrix_error(operation, status, &error));
        }
        Ok(body)
    }
}

/// 一条事件里算提醒要用的；缺事件 ID、发送人或者时间的跳过。不加密的房间看
/// `content.preview.conversation.mentions` 里有没有他。
fn reminder_message(
    raw: &Value,
    reader: &MatrixUserId,
    end_to_end_encrypted: bool,
) -> Option<ReminderMessage> {
    let event_id = raw.get("event_id").and_then(Value::as_str)?;
    let sender = raw
        .get("sender")
        .and_then(Value::as_str)
        .and_then(|sender| MatrixUserId::new(sender).ok())?;
    let sent_at = raw
        .get("origin_server_ts")
        .and_then(Value::as_i64)
        .and_then(|at| UtcMillis::new(at).ok())?;
    let mentions_reader = !end_to_end_encrypted
        && raw
            .pointer("/content/preview/conversation/mentions")
            .and_then(Value::as_array)
            .is_some_and(|mentions| {
                mentions
                    .iter()
                    .any(|mention| mention.as_str() == Some(reader.as_str()))
            });
    Some(ReminderMessage {
        event_id: event_id.to_owned(),
        sender,
        sent_at,
        mentions_reader,
    })
}

impl ReminderMatrixReader for MatrixApplicationServiceProvisioner {
    fn personal_workspace<'a>(
        &'a self,
        user: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<Option<Value>>> {
        Box::pin(self.personal_workspace_internal(user))
    }

    fn messages_since<'a>(
        &'a self,
        user: &'a MatrixUserId,
        room: &'a MatrixRoomId,
        end_to_end_encrypted: bool,
        since: UtcMillis,
    ) -> PortFuture<'a, MatrixResult<ReminderMessagePage>> {
        Box::pin(self.messages_since_internal(user, room, end_to_end_encrypted, since))
    }
}

#[cfg(test)]
#[path = "reminders/tests.rs"]
mod tests;
