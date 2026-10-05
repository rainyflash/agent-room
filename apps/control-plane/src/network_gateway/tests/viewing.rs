//! 按需查看（`specs/agent-reading/design.md` 第 5 步）：消息记录、按 ID 取、看前后、往前翻，
//! 以及回复更早的消息时从消息记录里补上摘录。

use std::time::Duration;

use agent_room_application::ports::MatrixTimelineEvent;
use serde_json::Value;
use uuid::Uuid;

use super::{
    Harness, MatrixSyncBatch, NetworkAgentMessaging, NetworkGatewayFailure, OTHER_AGENT, OWN_AGENT,
    ROOM, SECOND_ROOM, Step, TOKEN, batch, chat, chat_with, everything, harness, harness_in,
    in_room, matrix_user, other, own, revision, rooms_batch, texts,
};
use crate::network_gateway::{NetworkAgentRoomMessagesRequest, NetworkAgentRoomQuery};

/// 同步一次，记下这一批；交出来的不确认。
async fn sync(harness: &Harness, step: MatrixSyncBatch) -> Vec<Value> {
    harness.matrix.push(Step::Batch(Ok(step)));
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 50))
        .await
        .expect("同步")
        .messages
}

/// 别人说的一句；长正文只拿开头当标题和摘要，像真实发送方那样。
fn said(event_id: &str, text: &str) -> MatrixTimelineEvent {
    chat_with(event_id, other(), Uuid::now_v7(), text, &[], None)
}

fn event_ids(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|message| message["eventId"].as_str().unwrap())
        .collect()
}

fn history(before: Option<&str>, after: Option<&str>, limit: u16) -> NetworkAgentRoomQuery {
    NetworkAgentRoomQuery::History {
        before: before.map(str::to_owned),
        after: after.map(str::to_owned),
        limit,
        from: None,
        mentions_me: false,
    }
}

async fn view(
    harness: &Harness,
    room: Option<&str>,
    query: NetworkAgentRoomQuery,
) -> Result<(Vec<Value>, Option<String>), NetworkGatewayFailure> {
    harness
        .gateway
        .room_messages(
            TOKEN,
            NetworkAgentRoomMessagesRequest {
                room: room.map(str::to_owned),
                query,
            },
        )
        .await
        .map(|page| (page.messages, page.next_cursor))
}

#[tokio::test(start_paused = true)]
async fn 自己发的和确认过的都记着_按_id_取全文_同一条只给一次() {
    let harness = harness();
    let hello = Uuid::now_v7();
    let long = "字".repeat(1_500);
    sync(
        &harness,
        batch(
            "s1",
            vec![
                chat("$hello:matrix.test", other(), hello, "你好", [1; 64]),
                chat(
                    "$mine:matrix.test",
                    own(),
                    Uuid::now_v7(),
                    "我自己说的",
                    [1; 64],
                ),
                said("$long:matrix.test", &long),
            ],
        ),
    )
    .await;
    harness
        .gateway
        .acknowledge(TOKEN, "$long:matrix.test", None)
        .await
        .unwrap();

    let found = harness
        .gateway
        .get_messages(
            TOKEN,
            vec![
                "$mine:matrix.test".to_owned(),
                hello.to_string(),
                "$long:matrix.test".to_owned(),
                "$nope:matrix.test".to_owned(),
                "$hello:matrix.test".to_owned(),
            ],
        )
        .await
        .unwrap();

    assert_eq!(
        event_ids(&found.messages),
        [
            "$mine:matrix.test",
            "$hello:matrix.test",
            "$long:matrix.test"
        ]
    );
    assert_eq!(found.messages[0]["fromMe"], true);
    // 按 ID 取给全文，不截断。
    assert_eq!(found.messages[2]["conversation"]["text"], long.as_str());
    assert!(found.messages[2]["conversation"].get("truncated").is_none());
    assert_eq!(found.missing, ["$nope:matrix.test"]);
}

#[tokio::test(start_paused = true)]
async fn 不在所在房间里的消息算找不到() {
    let harness = harness();
    sync(
        &harness,
        rooms_batch(
            "s1",
            vec![
                (ROOM, vec![said("$here:matrix.test", "这里")]),
                (
                    SECOND_ROOM,
                    vec![in_room(&said("$there:matrix.test", "那边"), SECOND_ROOM)],
                ),
            ],
        ),
    )
    .await;

    let found = harness
        .gateway
        .get_messages(
            TOKEN,
            vec![
                "$here:matrix.test".to_owned(),
                "$there:matrix.test".to_owned(),
            ],
        )
        .await
        .unwrap();
    assert_eq!(event_ids(&found.messages), ["$here:matrix.test"]);
    assert_eq!(found.missing, ["$there:matrix.test"]);
}

#[tokio::test(start_paused = true)]
async fn 回复更早的消息时从消息记录里补上摘录_回复我发过的也算提到我() {
    let harness = harness();
    let mine = Uuid::now_v7();
    let theirs = Uuid::now_v7();
    sync(
        &harness,
        batch(
            "s1",
            vec![
                chat("$mine:matrix.test", own(), mine, "我先来", [1; 64]),
                chat(
                    "$theirs:matrix.test",
                    other(),
                    theirs,
                    "Ranger 的提议",
                    [1; 64],
                ),
            ],
        ),
    )
    .await;
    harness
        .gateway
        .acknowledge(TOKEN, "$theirs:matrix.test", None)
        .await
        .unwrap();

    let replies = sync(
        &harness,
        batch(
            "s2",
            vec![
                chat_with(
                    "$to-me:matrix.test",
                    other(),
                    Uuid::now_v7(),
                    "同意你",
                    &[],
                    Some(mine),
                ),
                chat_with(
                    "$to-them:matrix.test",
                    other(),
                    Uuid::now_v7(),
                    "补充一句",
                    &[],
                    Some(theirs),
                ),
            ],
        ),
    )
    .await;

    assert_eq!(texts(&replies), ["同意你", "补充一句"]);
    assert_eq!(replies[0]["replyTo"]["messageId"], mine.to_string());
    assert_eq!(replies[0]["replyTo"]["actorName"], "Scout");
    assert_eq!(replies[0]["replyTo"]["excerpt"], "我先来");
    assert_eq!(replies[0]["mentionsMe"], true, "回复的是我发的");
    assert_eq!(replies[1]["replyTo"]["actorName"], "Ranger");
    assert_eq!(replies[1]["replyTo"]["excerpt"], "Ranger 的提议");
    assert_eq!(replies[1]["mentionsMe"], false);
}

/// 大厅里五条（最后一条很长），第二个房间一条。
async fn five_in_lobby(harness: &Harness, long: &str) {
    sync(
        harness,
        rooms_batch(
            "s1",
            vec![
                (
                    ROOM,
                    vec![
                        said("$m1:matrix.test", "一"),
                        said("$m2:matrix.test", "二"),
                        said("$m3:matrix.test", "三"),
                        said("$m4:matrix.test", "四"),
                        said("$m5:matrix.test", long),
                    ],
                ),
                (
                    SECOND_ROOM,
                    vec![in_room(&said("$aside:matrix.test", "另一间"), SECOND_ROOM)],
                ),
            ],
        ),
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn 看前后_往前翻_往后翻都只看这个房间_长消息只给开头() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
    let long = "长".repeat(1_500);
    five_in_lobby(&harness, &long).await;

    let around = NetworkAgentRoomQuery::Around {
        id: "$m3:matrix.test".to_owned(),
        before: 1,
        after: 1,
    };
    let (messages, cursor) = view(&harness, Some(ROOM), around).await.unwrap();
    assert_eq!(
        event_ids(&messages),
        ["$m2:matrix.test", "$m3:matrix.test", "$m4:matrix.test"]
    );
    assert_eq!(cursor, None);

    // 往前翻：新的在前，接着翻把 nextCursor 当 before；翻到头就没有 nextCursor。
    let (messages, cursor) = view(&harness, Some(ROOM), history(None, None, 2))
        .await
        .unwrap();
    assert_eq!(event_ids(&messages), ["$m5:matrix.test", "$m4:matrix.test"]);
    assert_eq!(messages[0]["conversation"]["truncated"], true);
    assert_eq!(messages[0]["conversation"]["fullLength"], 1_500);
    assert_eq!(cursor.as_deref(), Some("$m4:matrix.test"));
    let (messages, cursor) = view(&harness, Some(ROOM), history(cursor.as_deref(), None, 2))
        .await
        .unwrap();
    assert_eq!(event_ids(&messages), ["$m3:matrix.test", "$m2:matrix.test"]);
    let (messages, cursor) = view(&harness, Some(ROOM), history(cursor.as_deref(), None, 2))
        .await
        .unwrap();
    assert_eq!(event_ids(&messages), ["$m1:matrix.test"]);
    assert_eq!(cursor, None);

    // 往后翻：旧的在前。
    let (messages, cursor) = view(
        &harness,
        Some(ROOM),
        history(None, Some("$m2:matrix.test"), 10),
    )
    .await
    .unwrap();
    assert_eq!(
        event_ids(&messages),
        ["$m3:matrix.test", "$m4:matrix.test", "$m5:matrix.test"]
    );
    assert_eq!(cursor, None);
}

#[tokio::test(start_paused = true)]
async fn 在几个房间里要说看哪间_别的房间的那条在这里算找不到() {
    let harness = harness_in(&[ROOM, SECOND_ROOM]);
    five_in_lobby(&harness, "五").await;

    assert_eq!(
        view(&harness, None, history(None, None, 5)).await,
        Err(NetworkGatewayFailure::RoomRequired)
    );
    let elsewhere = NetworkAgentRoomQuery::Around {
        id: "$aside:matrix.test".to_owned(),
        before: 1,
        after: 1,
    };
    assert_eq!(
        view(&harness, Some(ROOM), elsewhere).await,
        Err(NetworkGatewayFailure::MessageNotFound)
    );
    assert_eq!(
        view(
            &harness,
            Some(ROOM),
            history(Some("$aside:matrix.test"), None, 5)
        )
        .await,
        Err(NetworkGatewayFailure::MessageNotFound)
    );
    assert_eq!(
        view(
            &harness,
            Some("!elsewhere:matrix.test"),
            history(None, None, 5)
        )
        .await,
        Err(NetworkGatewayFailure::RoomNotJoined)
    );
}

#[tokio::test(start_paused = true)]
async fn 往前翻可以只看某个人_不分大小写_也可以只看提到我的() {
    let harness = harness();
    sync(
        &harness,
        batch(
            "s1",
            vec![
                said("$ranger:matrix.test", "Ranger 说"),
                chat(
                    "$scout:matrix.test",
                    own(),
                    Uuid::now_v7(),
                    "Scout 说",
                    [1; 64],
                ),
                chat_with(
                    "$named:matrix.test",
                    other(),
                    Uuid::now_v7(),
                    "Scout 你看呢",
                    &[matrix_user(OWN_AGENT)],
                    None,
                ),
            ],
        ),
    )
    .await;

    let from = |from: &str| NetworkAgentRoomQuery::History {
        before: None,
        after: None,
        limit: 10,
        from: Some(from.to_owned()),
        mentions_me: false,
    };
    let (messages, _) = view(&harness, None, from("ranger")).await.unwrap();
    assert_eq!(
        event_ids(&messages),
        ["$named:matrix.test", "$ranger:matrix.test"]
    );
    let (messages, _) = view(&harness, None, from(&matrix_user(OTHER_AGENT)))
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
    let (messages, _) = view(&harness, None, from(" Scout ")).await.unwrap();
    assert_eq!(event_ids(&messages), ["$scout:matrix.test"]);

    let mentions = NetworkAgentRoomQuery::History {
        before: None,
        after: None,
        limit: 10,
        from: None,
        mentions_me: true,
    };
    let (messages, _) = view(&harness, None, mentions).await.unwrap();
    assert_eq!(event_ids(&messages), ["$named:matrix.test"]);
}

#[tokio::test(start_paused = true)]
async fn 改过的消息记录跟着改_撤回的就取不到了() {
    let harness = harness();
    let kept = Uuid::now_v7();
    let gone = Uuid::now_v7();
    sync(
        &harness,
        batch(
            "s1",
            vec![
                chat("$kept:matrix.test", other(), kept, "原来的话", [1; 64]),
                chat("$gone:matrix.test", other(), gone, "要撤回的话", [1; 64]),
            ],
        ),
    )
    .await;
    harness
        .gateway
        .acknowledge(TOKEN, "$gone:matrix.test", None)
        .await
        .unwrap();
    sync(
        &harness,
        batch(
            "s2",
            vec![
                revision("$edit:matrix.test", other(), kept, "replace"),
                revision("$redact:matrix.test", other(), gone, "redact"),
            ],
        ),
    )
    .await;

    let found = harness
        .gateway
        .get_messages(
            TOKEN,
            vec![
                "$kept:matrix.test".to_owned(),
                "$gone:matrix.test".to_owned(),
            ],
        )
        .await
        .unwrap();
    assert_eq!(texts(&found.messages), ["改过的话"]);
    assert_eq!(found.missing, ["$gone:matrix.test"]);
}

#[tokio::test(start_paused = true)]
async fn 每个房间只留最近_500_条() {
    let harness = harness();
    let events: Vec<MatrixTimelineEvent> = (0..505)
        .map(|index| said(&format!("$m{index}:matrix.test"), &format!("第 {index} 条")))
        .collect();
    sync(&harness, batch("s1", events)).await;

    let found = harness
        .gateway
        .get_messages(
            TOKEN,
            vec!["$m4:matrix.test".to_owned(), "$m5:matrix.test".to_owned()],
        )
        .await
        .unwrap();
    assert_eq!(event_ids(&found.messages), ["$m5:matrix.test"]);
    assert_eq!(found.missing, ["$m4:matrix.test"]);
}

#[tokio::test(start_paused = true)]
async fn 参数不对时指出是哪一项() {
    let harness = harness();
    for ids in [
        Vec::new(),
        vec!["$a:matrix.test".to_owned(); 21],
        vec!["不是 ID".to_owned()],
        // UUIDv4 不是消息 ID。
        vec!["2f1c3a5e-8b7d-4c2a-9e6f-1a2b3c4d5e6f".to_owned()],
    ] {
        assert_eq!(
            harness.gateway.get_messages(TOKEN, ids).await,
            Err(NetworkGatewayFailure::InvalidLookup("ids"))
        );
    }
    let blank = NetworkAgentRoomQuery::History {
        before: None,
        after: None,
        limit: 10,
        from: Some("  ".to_owned()),
        mentions_me: false,
    };
    assert_eq!(
        view(&harness, None, blank).await,
        Err(NetworkGatewayFailure::InvalidLookup("from"))
    );
    let bad_anchor = NetworkAgentRoomQuery::Around {
        id: "nope".to_owned(),
        before: 1,
        after: 1,
    };
    assert_eq!(
        view(&harness, None, bad_anchor).await,
        Err(NetworkGatewayFailure::InvalidLookup("id"))
    );
}

#[test]
fn 看前后时_limit_条前后各一半_前面多给一条_每边最多二十条() {
    let around = |limit| {
        NetworkAgentRoomQuery::parse(
            Some("$m:matrix.test".to_owned()),
            None,
            None,
            limit,
            None,
            false,
        )
    };
    for (limit, before, after) in [(1, 1, 0), (5, 3, 2), (20, 10, 10), (50, 20, 20)] {
        assert_eq!(
            around(limit),
            Ok(NetworkAgentRoomQuery::Around {
                id: "$m:matrix.test".to_owned(),
                before,
                after,
            }),
            "{limit}"
        );
    }
    assert_eq!(around(0), Err("limit"));
    assert_eq!(around(51), Err("limit"));
}
