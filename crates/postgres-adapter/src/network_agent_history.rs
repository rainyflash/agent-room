//! 网络 Agent 的消息记录（`specs/agent-reading/design.md` 第 5 步“按需查看”）：每个房间留最近
//! 几百条，确认过的、它自己发的也在。写入随收件箱一起做（`network_agent_inbox.rs`），这里只读。

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        MatrixEventId, MatrixRoomId, NetworkAgentHistoryDirection, NetworkAgentHistoryFilter,
        NetworkAgentHistorySender, NetworkAgentMessageHistory, NetworkAgentMessageRef,
        NetworkAgentStoredMessage, PortFuture,
    },
};
use agent_room_domain::ids::{MessageId, NetworkAgentId};

use crate::{PostgresRepositories, agents::corrupt_data, error::map_sqlx_error};

type StoredRow = (i64, String, String, uuid::Uuid, String);

impl NetworkAgentMessageHistory for PostgresRepositories {
    fn messages_by_id<'a>(
        &'a self,
        id: NetworkAgentId,
        refs: &'a [NetworkAgentMessageRef],
    ) -> PortFuture<'a, RepositoryResult<Vec<NetworkAgentStoredMessage>>> {
        Box::pin(async move {
            let operation = "network_agent.history_by_id";
            let mut events: Vec<&str> = Vec::new();
            let mut messages: Vec<uuid::Uuid> = Vec::new();
            for reference in refs {
                match reference {
                    NetworkAgentMessageRef::Event(event) => events.push(event.as_str()),
                    NetworkAgentMessageRef::Message(message) => messages.push(message.as_uuid()),
                }
            }
            let rows = sqlx::query_as::<_, StoredRow>(
                r"SELECT sequence, matrix_event_id, matrix_room_id, message_id, preview::text
                    FROM agent_room.network_agent_message
                   WHERE network_agent_id = $1
                     AND (matrix_event_id = ANY($2) OR message_id = ANY($3))
                   ORDER BY sequence",
            )
            .bind(id.as_uuid())
            .bind(&events)
            .bind(&messages)
            .fetch_all(self.pool())
            .await
            .map_err(|error| map_sqlx_error(operation, &error))?;
            stored_messages(rows, operation)
        })
    }

    fn room_messages<'a>(
        &'a self,
        id: NetworkAgentId,
        room: &'a MatrixRoomId,
        direction: NetworkAgentHistoryDirection,
        filter: &'a NetworkAgentHistoryFilter,
        limit: u16,
    ) -> PortFuture<'a, RepositoryResult<Vec<NetworkAgentStoredMessage>>> {
        Box::pin(async move {
            let operation = "network_agent.history_room";
            let (sender_id, sender_name) = match &filter.from {
                Some(NetworkAgentHistorySender::MatrixUserId(user)) => (Some(user.as_str()), None),
                Some(NetworkAgentHistorySender::NameFolded(name)) => (None, Some(name.as_str())),
                None => (None, None),
            };
            // 往前翻新的在前，往后翻旧的在前；两种各用一条语句，好让它顺着索引取。
            let (statement, cursor) = match direction {
                NetworkAgentHistoryDirection::Before(before) => (
                    r"SELECT sequence, matrix_event_id, matrix_room_id, message_id, preview::text
                        FROM agent_room.network_agent_message
                       WHERE network_agent_id = $1 AND matrix_room_id = $2
                         AND ($3::bigint IS NULL OR sequence < $3)
                         AND (NOT $4 OR mentions_me)
                         AND ($5::text IS NULL OR actor_matrix_user_id = $5)
                         AND ($6::text IS NULL OR actor_name_folded = $6)
                       ORDER BY sequence DESC
                       LIMIT $7",
                    before,
                ),
                NetworkAgentHistoryDirection::After(after) => (
                    r"SELECT sequence, matrix_event_id, matrix_room_id, message_id, preview::text
                        FROM agent_room.network_agent_message
                       WHERE network_agent_id = $1 AND matrix_room_id = $2
                         AND sequence > $3
                         AND (NOT $4 OR mentions_me)
                         AND ($5::text IS NULL OR actor_matrix_user_id = $5)
                         AND ($6::text IS NULL OR actor_name_folded = $6)
                       ORDER BY sequence
                       LIMIT $7",
                    Some(after),
                ),
            };
            let cursor = cursor
                .map(i64::try_from)
                .transpose()
                .map_err(|_| corrupt_data(operation))?;
            let rows = sqlx::query_as::<_, StoredRow>(statement)
                .bind(id.as_uuid())
                .bind(room.as_str())
                .bind(cursor)
                .bind(filter.mentions_me)
                .bind(sender_id)
                .bind(sender_name)
                .bind(i64::from(limit))
                .fetch_all(self.pool())
                .await
                .map_err(|error| map_sqlx_error(operation, &error))?;
            stored_messages(rows, operation)
        })
    }
}

fn stored_messages(
    rows: Vec<StoredRow>,
    operation: &'static str,
) -> RepositoryResult<Vec<NetworkAgentStoredMessage>> {
    rows.into_iter()
        .map(|(sequence, event_id, room_id, message_id, preview)| {
            Ok(NetworkAgentStoredMessage {
                sequence: u64::try_from(sequence).map_err(|_| corrupt_data(operation))?,
                event_id: MatrixEventId::new(event_id).map_err(|_| corrupt_data(operation))?,
                room_id: MatrixRoomId::new(room_id).map_err(|_| corrupt_data(operation))?,
                message_id: MessageId::from_uuid(message_id),
                preview: serde_json::from_str(&preview).map_err(|_| corrupt_data(operation))?,
            })
        })
        .collect()
}
