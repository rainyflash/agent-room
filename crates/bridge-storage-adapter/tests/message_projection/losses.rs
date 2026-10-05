//! 补不回来的一段（`specs/agent-reading/design.md` 第 4 步的 `gaps`）。

use agent_room_application::ports::MatrixBackfillToken;
use agent_room_bridge_core::messages::{
    MessageBackfillBatch, MessageProjectionBatch, MessageProjectionMutation, MessageTimelineGap,
    MessageTimelineProjectionStore as _, MessageTimelineQueryRepository as _, TimelineLoss,
    TimelineLossReason,
};
use agent_room_bridge_storage_adapter::SqliteMessageTimelineRepository;
use agent_room_domain::ids::MessageId;
use uuid::Uuid;

use super::{event_id, open_store, owner_actor, preview_mutation, room_id, sync_token};

fn message(event: &str, at: i64) -> MessageProjectionMutation {
    let server_time = u64::try_from(at).expect("时间不是负数");
    preview_mutation(
        event,
        MessageId::from_uuid(Uuid::now_v7()),
        owner_actor(),
        at,
        "消息",
        1,
        Some(server_time),
    )
}

/// 这次同步这个房间有没有缺口。
#[derive(Clone, Copy)]
enum Gap {
    No,
    /// 有，能从这个令牌往回补。
    Token(&'static str),
    /// 有，但没给往回翻的令牌。
    NoToken,
}

/// 同步一批。
async fn sync(
    store: &SqliteMessageTimelineRepository,
    token: &str,
    messages: Vec<MessageProjectionMutation>,
    gap_kind: Gap,
) {
    let gap = |previous_batch| MessageTimelineGap {
        room_id: room_id(),
        previous_batch,
    };
    let gaps = match gap_kind {
        Gap::No => Vec::new(),
        Gap::Token(token) => vec![gap(Some(
            MatrixBackfillToken::new(token).expect("回填游标有效"),
        ))],
        Gap::NoToken => vec![gap(None)],
    };
    store
        .apply(&MessageProjectionBatch::new(
            sync_token(token),
            messages,
            Vec::new(),
            gaps,
        ))
        .await
        .expect("同步批次可写入");
}

async fn backfill(
    store: &SqliteMessageTimelineRepository,
    messages: Vec<MessageProjectionMutation>,
    complete: bool,
) {
    let gap = store
        .pending_gaps(16)
        .await
        .expect("可查询")
        .pop()
        .expect("有一段缺口");
    store
        .apply_backfill(&MessageBackfillBatch::new(
            gap,
            messages,
            Vec::new(),
            complete,
        ))
        .await
        .expect("补回的事件可写入");
}

async fn gaps_at(store: &SqliteMessageTimelineRepository, events: &[&str]) -> Vec<TimelineLoss> {
    let ids = events
        .iter()
        .map(|event| event_id(event))
        .collect::<Vec<_>>();
    store.inbox_gaps(&room_id(), &ids).await.expect("可查询")
}

#[tokio::test]
async fn 往回补没接上时记下丢了的一段_读到它后面那条时才报() {
    let (_temporary, store, _inspector) = open_store().await;
    sync(
        &store,
        "sync-1",
        vec![message("$old:matrix.test", 1_000)],
        Gap::No,
    )
    .await;
    sync(
        &store,
        "sync-2",
        vec![message("$latest:matrix.test", 5_000)],
        Gap::Token("backfill-gap"),
    )
    .await;
    backfill(&store, vec![message("$missed:matrix.test", 4_000)], false).await;

    // 丢的是 $old 和补回来的最早一条 $missed 之间。
    assert_eq!(
        gaps_at(&store, &["$latest:matrix.test", "$missed:matrix.test"]).await,
        [TimelineLoss {
            after_event_id: Some(event_id("$old:matrix.test")),
            before_event_id: event_id("$missed:matrix.test"),
            reason: TimelineLossReason::TooMany,
        }]
    );
    assert!(gaps_at(&store, &["$latest:matrix.test"]).await.is_empty());
}

#[tokio::test]
async fn 一条也没补回来时_丢的一段挂在当时同步来的第一条前面() {
    let (_temporary, store, _inspector) = open_store().await;
    sync(
        &store,
        "sync-1",
        vec![message("$old:matrix.test", 1_000)],
        Gap::No,
    )
    .await;
    sync(
        &store,
        "sync-2",
        vec![
            message("$first:matrix.test", 5_000),
            message("$second:matrix.test", 6_000),
        ],
        Gap::Token("backfill-gap"),
    )
    .await;
    backfill(&store, Vec::new(), false).await;

    assert_eq!(
        gaps_at(&store, &["$first:matrix.test", "$second:matrix.test"]).await,
        [TimelineLoss {
            after_event_id: Some(event_id("$old:matrix.test")),
            before_event_id: event_id("$first:matrix.test"),
            reason: TimelineLossReason::TooMany,
        }]
    );
}

#[tokio::test]
async fn 接上了不算丢_没法往回补的一段同步时就记下() {
    let (_temporary, store, _inspector) = open_store().await;
    sync(
        &store,
        "sync-1",
        vec![message("$old:matrix.test", 1_000)],
        Gap::No,
    )
    .await;
    sync(
        &store,
        "sync-2",
        vec![message("$latest:matrix.test", 5_000)],
        Gap::Token("backfill-gap"),
    )
    .await;
    backfill(&store, vec![message("$missed:matrix.test", 4_000)], true).await;
    assert!(
        gaps_at(&store, &["$latest:matrix.test", "$missed:matrix.test"])
            .await
            .is_empty()
    );

    // 没有往回翻的令牌：补不了。之前按时间最新的是 $latest（补回来的 $missed 收得晚，时间却更早）。
    sync(
        &store,
        "sync-3",
        vec![message("$after-gap:matrix.test", 9_000)],
        Gap::NoToken,
    )
    .await;
    assert!(store.pending_gaps(16).await.expect("可查询").is_empty());
    assert_eq!(
        gaps_at(&store, &["$after-gap:matrix.test"]).await,
        [TimelineLoss {
            after_event_id: Some(event_id("$latest:matrix.test")),
            before_event_id: event_id("$after-gap:matrix.test"),
            reason: TimelineLossReason::TooMany,
        }]
    );
}
