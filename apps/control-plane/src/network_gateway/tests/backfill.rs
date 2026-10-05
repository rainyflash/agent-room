//! 同步时一个房间一次来得太多就往回补，补不回来的报 `gaps`（`specs/agent-reading/design.md`
//! 第 5 步）。

use std::time::Duration;

use agent_room_application::ports::{
    MatrixBackfillToken, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixRoomId,
    MatrixRoomSync, MatrixRoomSyncKind, MatrixSyncBatch, MatrixSyncToken, MatrixTimelineEvent,
};
use agent_room_bridge_ipc::{
    IpcTimelineGap,
    wake::{WaitOptions, WakeRule},
};
use agent_room_domain::time::UtcMillis;
use serde_json::Value;
use uuid::Uuid;

use super::{
    Harness, NetworkAgentMessaging, NetworkAgentWait, NetworkGatewayFailure, OWN_AGENT, ROOM,
    SECOND_ROOM, Step, TOKEN, backfill_page, batch, chat_with, everything, harness, harness_in,
    in_room, matrix_user, other, own,
};
use crate::network_gateway::NetworkAgentMessages;

/// 别人说的一句，正文就是事件 ID，好认。
fn said(event_id: &str) -> MatrixTimelineEvent {
    chat_with(event_id, other(), Uuid::now_v7(), event_id, &[], None)
}

/// 别人点我的一句。
fn named(event_id: &str) -> MatrixTimelineEvent {
    chat_with(
        event_id,
        other(),
        Uuid::now_v7(),
        event_id,
        &[matrix_user(OWN_AGENT)],
        None,
    )
}

/// 一个房间被截断的同步：只带最近这几条，`previous` 是往回翻的令牌。
fn limited(
    next: &str,
    room: &str,
    previous: Option<&str>,
    events: Vec<MatrixTimelineEvent>,
) -> MatrixSyncBatch {
    MatrixSyncBatch::new(
        MatrixSyncToken::new(next).unwrap(),
        vec![MatrixRoomSync::new(
            MatrixRoomId::new(room).unwrap(),
            MatrixRoomSyncKind::Joined,
            true,
            previous.map(|token| MatrixBackfillToken::new(token).unwrap()),
            events,
            Vec::new(),
        )],
    )
}

fn event_ids(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|message| message["eventId"].as_str().unwrap())
        .collect()
}

fn gap(after: Option<&str>, before: &str) -> IpcTimelineGap {
    IpcTimelineGap {
        room_id: ROOM.to_owned(),
        after_event_id: after.map(str::to_owned),
        before_event_id: before.to_owned(),
        reason: "too_many".to_owned(),
    }
}

/// 有什么给什么，一次最多 `limit` 条；交出来的不确认。
async fn take(
    harness: &Harness,
    limit: u16,
) -> Result<NetworkAgentMessages, NetworkGatewayFailure> {
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, limit))
        .await
}

async fn acknowledge(harness: &Harness, event_id: &str) {
    harness
        .gateway
        .acknowledge(TOKEN, event_id, None)
        .await
        .unwrap();
}

/// 先同步一次：大厅里有 `$seen`，交出来以后确认掉。之后的同步就是“已经在跟这个大厅”。
async fn settled(harness: &Harness) {
    harness.matrix.push(Step::Batch(Ok(batch(
        "s1",
        vec![said("$seen:matrix.test")],
    ))));
    let first = take(harness, 50).await.unwrap();
    assert_eq!(event_ids(&first.messages), ["$seen:matrix.test"]);
    acknowledge(harness, "$seen:matrix.test").await;
}

#[tokio::test(start_paused = true)]
async fn 一次来得太多就往回补_接上记过的那条为止_补回来的排在前面一起交() {
    let harness = harness();
    settled(&harness).await;
    harness.matrix.push(Step::Batch(Ok(limited(
        "s2",
        ROOM,
        Some("p2"),
        vec![said("$m4:matrix.test"), said("$m5:matrix.test")],
    ))));
    // 第一页没接上，第二页碰到记过的那条就停，更早的不要。
    harness.matrix.backfill_with(Ok(backfill_page(
        "p2",
        Some("p1"),
        vec![said("$m3:matrix.test"), said("$m2:matrix.test")],
    )));
    harness.matrix.backfill_with(Ok(backfill_page(
        "p1",
        Some("p0"),
        vec![
            said("$m1:matrix.test"),
            said("$seen:matrix.test"),
            said("$older:matrix.test"),
        ],
    )));

    let page = take(&harness, 50).await.expect("取到消息");

    assert_eq!(
        event_ids(&page.messages),
        [
            "$m1:matrix.test",
            "$m2:matrix.test",
            "$m3:matrix.test",
            "$m4:matrix.test",
            "$m5:matrix.test"
        ]
    );
    assert!(page.gaps.is_empty(), "接上了，没有补不回来的");
    assert_eq!(
        harness.matrix.backfilled(),
        [
            (ROOM.to_owned(), "p2".to_owned(), 100),
            (ROOM.to_owned(), "p1".to_owned(), 100)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn 被截断却没给往回翻的令牌_更早的那段报成补不回来_确认之前每次交都说() {
    let harness = harness();
    settled(&harness).await;
    harness.matrix.push(Step::Batch(Ok(limited(
        "s2",
        ROOM,
        None,
        vec![said("$m4:matrix.test"), said("$m5:matrix.test")],
    ))));

    let page = take(&harness, 50).await.expect("取到消息");

    assert_eq!(
        event_ids(&page.messages),
        ["$m4:matrix.test", "$m5:matrix.test"]
    );
    assert_eq!(
        page.gaps,
        [gap(Some("$seen:matrix.test"), "$m4:matrix.test")]
    );
    assert!(harness.matrix.backfilled().is_empty());

    // 没确认，下次交出这几条时再说一遍；确认了就不再说。
    let again = take(&harness, 50).await.unwrap();
    assert_eq!(again.gaps, page.gaps);
    acknowledge(&harness, "$m5:matrix.test").await;
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s3", vec![said("$m6:matrix.test")]))));
    let later = take(&harness, 50).await.unwrap();
    assert_eq!(event_ids(&later.messages), ["$m6:matrix.test"]);
    assert!(later.gaps.is_empty());
}

#[tokio::test(start_paused = true)]
async fn 往回翻时不让读了_补到的照样交_更早的报成补不回来() {
    let harness = harness();
    settled(&harness).await;
    harness.matrix.push(Step::Batch(Ok(limited(
        "s2",
        ROOM,
        Some("p2"),
        vec![said("$m4:matrix.test")],
    ))));
    harness.matrix.backfill_with(Ok(backfill_page(
        "p2",
        Some("p1"),
        vec![said("$m3:matrix.test"), said("$m2:matrix.test")],
    )));
    harness.matrix.backfill_with(Err(MatrixFailure::new(
        MatrixOperation::Backfill,
        MatrixFailureKind::Forbidden,
    )));

    let page = take(&harness, 50).await.expect("取到消息");

    assert_eq!(
        event_ids(&page.messages),
        ["$m2:matrix.test", "$m3:matrix.test", "$m4:matrix.test"]
    );
    assert_eq!(
        page.gaps,
        [gap(Some("$seen:matrix.test"), "$m2:matrix.test")]
    );
}

#[tokio::test(start_paused = true)]
async fn 这个房间这一次凑够消息记录能留的条数就不再往回翻() {
    let harness = harness();
    settled(&harness).await;
    // 这次同步来了 450 条，最多再补 50 条就够一个房间留的 500 条。
    let synced: Vec<MatrixTimelineEvent> = (0..450)
        .map(|index| said(&format!("$new{index}:matrix.test")))
        .collect();
    harness
        .matrix
        .push(Step::Batch(Ok(limited("s2", ROOM, Some("p2"), synced))));
    let earlier: Vec<MatrixTimelineEvent> = (0..50)
        .rev()
        .map(|index| said(&format!("$old{index}:matrix.test")))
        .collect();
    harness
        .matrix
        .backfill_with(Ok(backfill_page("p2", Some("p1"), earlier)));

    let page = take(&harness, 50).await.expect("取到消息");

    assert_eq!(
        harness.matrix.backfilled(),
        [(ROOM.to_owned(), "p2".to_owned(), 50)],
        "只要还差的 50 条，凑够了不再翻"
    );
    assert_eq!(page.messages.len(), 50);
    assert_eq!(page.messages[0]["eventId"], "$old0:matrix.test");
    assert_eq!(
        page.gaps,
        [gap(Some("$seen:matrix.test"), "$old0:matrix.test")]
    );
    assert_eq!(page.remaining, 450);
    assert_eq!(page.dropped, 0, "收件箱每个房间也是 500 条，一条不丢");
}

#[tokio::test(start_paused = true)]
async fn 往回翻暂时读不到时这次同步不算数_下次从同一位置再补上() {
    let harness = harness();
    settled(&harness).await;
    let cut = || {
        limited(
            "s2",
            ROOM,
            Some("p2"),
            vec![said("$m3:matrix.test"), said("$m4:matrix.test")],
        )
    };
    harness.matrix.push(Step::Batch(Ok(cut())));
    harness.matrix.backfill_with(Err(MatrixFailure::new(
        MatrixOperation::Backfill,
        MatrixFailureKind::RateLimited,
    )));

    // 和同步失败一样：等着的报暂时不可用（只看一眼的照旧有什么给什么）。
    assert_eq!(
        harness
            .gateway
            .wait_for_messages(TOKEN, everything(Duration::from_secs(30), 50))
            .await
            .unwrap_err(),
        NetworkGatewayFailure::Unavailable
    );
    assert_eq!(
        harness.inbox.sync_token().as_deref(),
        Some("s1"),
        "位置没动"
    );

    harness.matrix.push(Step::Batch(Ok(cut())));
    harness.matrix.backfill_with(Ok(backfill_page(
        "p2",
        Some("p1"),
        vec![said("$m2:matrix.test"), said("$seen:matrix.test")],
    )));
    let page = take(&harness, 50).await.expect("取到消息");

    assert_eq!(
        event_ids(&page.messages),
        ["$m2:matrix.test", "$m3:matrix.test", "$m4:matrix.test"]
    );
    assert!(page.gaps.is_empty());
    let since: Vec<Option<String>> = harness
        .matrix
        .requests()
        .iter()
        .map(|request| {
            request
                .since
                .as_ref()
                .map(|token| token.as_str().to_owned())
        })
        .collect();
    assert_eq!(
        since,
        [None, Some("s1".to_owned()), Some("s1".to_owned())],
        "第二次从同一位置再同步"
    );
    assert_eq!(harness.inbox.sync_token().as_deref(), Some("s2"));
}

#[tokio::test(start_paused = true)]
async fn 第一次同步和刚进来的房间本来就只取最近一段_不往回补() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
    harness.matrix.push(Step::Batch(Ok(limited(
        "s1",
        ROOM,
        Some("p0"),
        vec![said("$seen:matrix.test")],
    ))));
    let first = take(&harness, 50).await.unwrap();
    assert_eq!(event_ids(&first.messages), ["$seen:matrix.test"]);
    acknowledge(&harness, "$seen:matrix.test").await;

    // 二号房的消息记录里还没有它的消息：刚进来的房间。
    harness.matrix.push(Step::Batch(Ok(limited(
        "s2",
        SECOND_ROOM,
        Some("q1"),
        vec![in_room(&said("$hello:matrix.test"), SECOND_ROOM)],
    ))));
    let page = take(&harness, 50).await.unwrap();

    assert_eq!(event_ids(&page.messages), ["$hello:matrix.test"]);
    assert!(page.gaps.is_empty());
    assert!(harness.matrix.backfilled().is_empty());
}

#[tokio::test(start_paused = true)]
async fn 进过加密房间的用加密客户端往回补() {
    let harness = harness();
    *harness.agents.encrypted_since.lock().unwrap() = Some(UtcMillis::new(1).unwrap());
    {
        let mut batches = harness.encrypted.batches.lock().unwrap();
        batches.push_back(Ok(batch("e1", vec![said("$seen:matrix.test")])));
        batches.push_back(Ok(limited(
            "e2",
            ROOM,
            Some("p2"),
            vec![said("$m2:matrix.test")],
        )));
    }
    harness
        .encrypted
        .backfills
        .lock()
        .unwrap()
        .push_back(Ok(backfill_page(
            "p2",
            Some("p1"),
            vec![said("$m1:matrix.test"), said("$seen:matrix.test")],
        )));
    let first = take(&harness, 50).await.unwrap();
    assert_eq!(event_ids(&first.messages), ["$seen:matrix.test"]);
    acknowledge(&harness, "$seen:matrix.test").await;

    let page = take(&harness, 50).await.expect("取到消息");

    assert_eq!(
        event_ids(&page.messages),
        ["$m1:matrix.test", "$m2:matrix.test"]
    );
    assert_eq!(
        *harness.encrypted.backfilled.lock().unwrap(),
        [(ROOM.to_owned(), "p2".to_owned(), 100)]
    );
    assert!(harness.matrix.backfilled().is_empty(), "轻量客户端不碰它");
}

#[tokio::test(start_paused = true)]
async fn 补不回来的一段跟着后面那条交_跳过的也算_那条还没交到就先不说() {
    let harness = harness();
    settled(&harness).await;
    harness
        .matrix
        .push(Step::Batch(Ok(batch("s2", vec![said("$m1:matrix.test")]))));
    let before = take(&harness, 1).await.unwrap();
    assert_eq!(event_ids(&before.messages), ["$m1:matrix.test"]);

    // 没确认 m1 时又来了一段被截断的：缺口记在 m2 上，在 m1 之后。
    harness.matrix.push(Step::Batch(Ok(limited(
        "s3",
        ROOM,
        None,
        vec![said("$m2:matrix.test"), named("$m3:matrix.test")],
    ))));
    let one = take(&harness, 1).await.unwrap();
    assert_eq!(event_ids(&one.messages), ["$m1:matrix.test"]);
    assert!(one.gaps.is_empty(), "m2 还没交到");

    // 只要点我的：m1、m2 算跳过，交 m3；m2 前面的缺口也算交到了。
    let mentions = NetworkAgentWait {
        options: WaitOptions {
            wake: WakeRule::Mentions,
            mentions_only: true,
            ..WaitOptions::default()
        },
        ..everything(Duration::ZERO, 20)
    };
    let named = harness
        .gateway
        .wait_for_messages(TOKEN, mentions)
        .await
        .unwrap();
    assert_eq!(event_ids(&named.messages), ["$m3:matrix.test"]);
    assert_eq!(named.skipped, 2);
    assert_eq!(
        named.gaps,
        [gap(Some("$m1:matrix.test"), "$m2:matrix.test")]
    );
}

#[tokio::test(start_paused = true)]
async fn 缺口之后只有自己发的_没处可挂就不说() {
    let harness = harness();
    settled(&harness).await;
    harness.matrix.push(Step::Batch(Ok(limited(
        "s2",
        ROOM,
        None,
        vec![chat_with(
            "$mine:matrix.test",
            own(),
            Uuid::now_v7(),
            "我自己说的",
            &[],
            None,
        )],
    ))));
    let mine = take(&harness, 50).await.unwrap();
    assert!(mine.messages.is_empty(), "自己发的不进收件箱");

    harness
        .matrix
        .push(Step::Batch(Ok(batch("s3", vec![said("$m2:matrix.test")]))));
    let page = take(&harness, 50).await.unwrap();

    assert_eq!(event_ids(&page.messages), ["$m2:matrix.test"]);
    assert!(page.gaps.is_empty());
}
