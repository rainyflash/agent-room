use std::{collections::HashSet, time::Duration};

use super::{
    Arrival, Delivery, IpcWake, MAX_PEOPLE, WaitDecision, WaitOptions, WaitOptionsField,
    WakeContext, WakeReason, WakeRule, decide, mentioned_people, wakes,
};
use crate::{
    IpcActorSummary, IpcAgentSummary, IpcContentReference, IpcConversationMessage,
    IpcMessagePreviewSummary, IpcMessageProvenance, IpcMessageSensitivity, IpcReplyExcerpt,
};

const ME: &str = "@scout:matrix.test";
const OWNER: &str = "@owner:matrix.test";
const ADA: &str = "@ada:matrix.test";
const BOB: &str = "@bob:matrix.test";
const NOVA: &str = "@nova:matrix.test";
const ROOM: &str = "!lobby:matrix.test";
const OTHER_ROOM: &str = "!other:matrix.test";
const DIRECT_ROOM: &str = "!direct:matrix.test";

fn message(actor: IpcActorSummary, text: &str) -> IpcMessagePreviewSummary {
    IpcMessagePreviewSummary {
        conversation: Some(IpcConversationMessage {
            attachment_name: None,
            text: text.to_owned(),
            mentions: Vec::new(),
            truncated: false,
            full_length: None,
        }),
        reply_to_message_id: None,
        reply_to: None,
        message_id: format!("m-{text}"),
        event_id: format!("${text}:matrix.test"),
        room_id: ROOM.to_owned(),
        actor,
        created_at_unix_ms: 1_000,
        title: "聊天".to_owned(),
        summary: "聊天".to_owned(),
        content: IpcContentReference {
            content_id: "content".to_owned(),
            digest_sha256: "0".repeat(64),
            media_type: "text/plain".to_owned(),
            size_bytes: 1,
        },
        language: None,
        sensitivity: IpcMessageSensitivity::Normal,
        risk_flags: Vec::new(),
        from_me: false,
        mentions_me: false,
    }
}

fn human(matrix_user_id: &str, text: &str) -> IpcMessagePreviewSummary {
    message(
        IpcActorSummary::Human {
            principal_id: format!("principal-{matrix_user_id}"),
            display_name: matrix_user_id.to_owned(),
            matrix_user_id: matrix_user_id.to_owned(),
            avatar_url: None,
        },
        text,
    )
}

fn agent(matrix_user_id: &str, text: &str) -> IpcMessagePreviewSummary {
    let mut preview = message(
        IpcActorSummary::Agent {
            agent: IpcAgentSummary {
                agent_id: format!("agent-{matrix_user_id}"),
                display_name: matrix_user_id.to_owned(),
                matrix_user_id: matrix_user_id.to_owned(),
                avatar_url: None,
            },
            instance_id: "instance".to_owned(),
            provenance: IpcMessageProvenance::AutonomousAgent,
        },
        text,
    );
    preview.from_me = matrix_user_id == ME;
    preview
}

fn mentioning(mut preview: IpcMessagePreviewSummary, people: &[&str]) -> IpcMessagePreviewSummary {
    if let Some(conversation) = &mut preview.conversation {
        conversation.mentions = people.iter().map(|&person| person.to_owned()).collect();
    }
    preview.mentions_me |= people.contains(&ME);
    preview
}

/// 回复一条消息；`author` 是被回复那条的作者，`None` 表示找不到那条。
fn replying(
    mut preview: IpcMessagePreviewSummary,
    target: &str,
    author: Option<&str>,
) -> IpcMessagePreviewSummary {
    preview.reply_to_message_id = Some(target.to_owned());
    preview.reply_to = author.map(|author| IpcReplyExcerpt {
        message_id: target.to_owned(),
        actor_name: author.to_owned(),
        excerpt: "原话".to_owned(),
    });
    preview.mentions_me |= author == Some(ME);
    preview
}

fn in_room(mut preview: IpcMessagePreviewSummary, room: &str) -> IpcMessagePreviewSummary {
    preview.room_id = room.to_owned();
    preview
}

fn context(direct_rooms: &HashSet<String>) -> WakeContext<'_> {
    WakeContext {
        owner: Some(OWNER),
        direct_rooms,
    }
}

fn arrivals(messages: &[(IpcMessagePreviewSummary, i64)]) -> Vec<Arrival<'_>> {
    messages
        .iter()
        .map(|(preview, arrived_at_ms)| Arrival {
            preview,
            arrived_at_ms: *arrived_at_ms,
        })
        .collect()
}

fn delivered(decision: WaitDecision) -> Delivery {
    match decision {
        WaitDecision::Deliver(delivery) => delivery,
        WaitDecision::Wait { recheck_at_ms } => panic!("应当交出去，却还在等（{recheck_at_ms:?}）"),
    }
}

fn rule(wake: WakeRule) -> WaitOptions {
    WaitOptions {
        wake,
        ..WaitOptions::default()
    }
}

#[test]
fn 跟我有关_人说的叫醒_点了别人或回复别人的不叫醒() {
    let direct = HashSet::new();
    let options = WaitOptions::default();
    let wakes_me = |preview: &IpcMessagePreviewSummary| wakes(preview, &options, context(&direct));

    assert!(wakes_me(&human(ADA, "大家好")));
    assert!(!wakes_me(&mentioning(human(ADA, "Bob 你看"), &[BOB])));
    assert!(!wakes_me(&replying(human(ADA, "好的"), "m-bob", Some(BOB))));
    assert!(wakes_me(&mentioning(
        human(ADA, "Scout 和 Bob"),
        &[ME, BOB]
    )));
    assert!(
        wakes_me(&replying(human(ADA, "那条呢"), "m-gone", None)),
        "找不到被回复的那条时宁可叫醒"
    );
}

#[test]
fn 跟我有关_agent_说的要点我或回复我_两人房间都算() {
    let direct = HashSet::from([DIRECT_ROOM.to_owned()]);
    let options = WaitOptions::default();
    let wakes_me = |preview: &IpcMessagePreviewSummary| wakes(preview, &options, context(&direct));

    assert!(!wakes_me(&agent(NOVA, "我来")), "Agent 之间没点我不叫醒");
    assert!(wakes_me(&mentioning(agent(NOVA, "Scout 你看"), &[ME])));
    assert!(wakes_me(&replying(agent(NOVA, "同意"), "m-mine", Some(ME))));
    assert!(wakes_me(&in_room(agent(NOVA, "私聊"), DIRECT_ROOM)));
}

#[test]
fn 主人说话总能叫醒_自己发的不叫醒() {
    let direct = HashSet::new();
    let owner_to_bob = mentioning(human(OWNER, "Bob 你来"), &[BOB]);
    for options in [
        WaitOptions::default(),
        rule(WakeRule::Mentions),
        WaitOptions {
            from: vec![ADA.to_owned()],
            ..WaitOptions::default()
        },
    ] {
        assert!(wakes(&owner_to_bob, &options, context(&direct)));
    }
    assert!(!wakes(
        &agent(ME, "我先来"),
        &rule(WakeRule::All),
        context(&direct)
    ));
}

#[test]
fn 只在点名时_和全部都叫醒() {
    let direct = HashSet::new();
    let mentions = rule(WakeRule::Mentions);
    assert!(!wakes(&human(ADA, "大家好"), &mentions, context(&direct)));
    assert!(wakes(
        &mentioning(human(ADA, "Scout？"), &[ME]),
        &mentions,
        context(&direct)
    ));
    assert!(wakes(
        &agent(NOVA, "随便说说"),
        &rule(WakeRule::All),
        context(&direct)
    ));
}

#[test]
fn 只等某些人或某条的回复时_别的都不叫醒() {
    let direct = HashSet::new();
    let from_ada = WaitOptions {
        from: vec![ADA.to_owned()],
        ..WaitOptions::default()
    };
    assert!(wakes(&human(ADA, "我回来了"), &from_ada, context(&direct)));
    assert!(!wakes(&human(BOB, "我也在"), &from_ada, context(&direct)));
    assert!(
        !wakes(
            &mentioning(agent(NOVA, "Scout？"), &[ME]),
            &from_ada,
            context(&direct)
        ),
        "只等 Ada 时，别人点名也先不叫醒"
    );

    let replies = WaitOptions {
        reply_to: Some("m-question".to_owned()),
        ..WaitOptions::default()
    };
    assert!(wakes(
        &replying(human(BOB, "答案"), "m-question", Some(ME)),
        &replies,
        context(&direct)
    ));
    assert!(!wakes(
        &replying(human(BOB, "别的"), "m-other", Some(ADA)),
        &replies,
        context(&direct)
    ));
}

#[test]
fn 防抖_等对话停_5_秒再交_期间来新消息就重新计时() {
    let direct = HashSet::new();
    let options = WaitOptions::default();
    let mut messages = vec![(human(ADA, "在吗"), 0)];
    assert_eq!(
        decide(
            &arrivals(&messages),
            &options,
            context(&direct),
            20,
            1_000,
            None
        ),
        WaitDecision::Wait {
            recheck_at_ms: Some(5_000)
        }
    );

    messages.push((human(ADA, "帮我看个东西"), 3_000));
    assert_eq!(
        decide(
            &arrivals(&messages),
            &options,
            context(&direct),
            20,
            4_000,
            None
        ),
        WaitDecision::Wait {
            recheck_at_ms: Some(8_000)
        }
    );

    let delivery = delivered(decide(
        &arrivals(&messages),
        &options,
        context(&direct),
        20,
        8_000,
        None,
    ));
    assert_eq!(delivery.picks, [0, 1]);
    assert_eq!(delivery.wake.reason, WakeReason::Messages);
    assert_eq!(
        delivery.wake.event_ids,
        ["$在吗:matrix.test", "$帮我看个东西:matrix.test"]
    );
}

#[test]
fn 一直有人说话时_从第一条叫醒它的算起最多多等_30_秒() {
    let direct = HashSet::new();
    let messages: Vec<_> = (0..8)
        .map(|step| (human(ADA, &format!("第 {step} 句")), step * 4_000))
        .collect();
    let options = WaitOptions::default();
    assert_eq!(
        decide(
            &arrivals(&messages),
            &options,
            context(&direct),
            20,
            29_000,
            None
        ),
        WaitDecision::Wait {
            recheck_at_ms: Some(30_000)
        }
    );
    let delivery = delivered(decide(
        &arrivals(&messages),
        &options,
        context(&direct),
        20,
        30_000,
        None,
    ));
    assert_eq!(delivery.picks.len(), 8);
}

#[test]
fn 立刻交的几种情况() {
    let direct = HashSet::new();
    let fresh = [(human(ADA, "在吗"), 10_000), (human(BOB, "我也在"), 10_000)];
    let immediately = WaitOptions {
        settle: Duration::ZERO,
        ..WaitOptions::default()
    };
    let now = 10_100;
    let decide_now = |options: &WaitOptions, limit: usize, deadline: Option<i64>| {
        decide(
            &arrivals(&fresh),
            options,
            context(&direct),
            limit,
            now,
            deadline,
        )
    };

    assert!(matches!(
        decide_now(&immediately, 20, None),
        WaitDecision::Deliver(_)
    ));
    let default = WaitOptions::default();
    assert!(
        matches!(decide_now(&default, 2, None), WaitDecision::Deliver(_)),
        "一批攒满了"
    );
    assert!(
        matches!(
            decide_now(&default, 20, Some(now)),
            WaitDecision::Deliver(_)
        ),
        "这次等消息的时间到了"
    );

    let stale = [(human(ADA, "早就说了"), 0)];
    assert!(
        matches!(
            decide(
                &arrivals(&stale),
                &default,
                context(&direct),
                20,
                60_000,
                None
            ),
            WaitDecision::Deliver(_)
        ),
        "早就安静了的积压不用再等"
    );
}

#[test]
fn 没有叫醒它的消息时_等满时间空手返回_消息留着() {
    let direct = HashSet::new();
    let messages = [(agent(NOVA, "我们俩先聊"), 0)];
    let options = WaitOptions::default();
    assert_eq!(
        decide(
            &arrivals(&messages),
            &options,
            context(&direct),
            20,
            10_000,
            Some(30_000)
        ),
        WaitDecision::Wait {
            recheck_at_ms: None
        }
    );
    assert_eq!(
        delivered(decide(
            &arrivals(&messages),
            &options,
            context(&direct),
            20,
            30_000,
            Some(30_000),
        )),
        Delivery {
            picks: Vec::new(),
            skipped: 0,
            remaining: 1,
            wake: IpcWake::empty(WakeReason::Timeout),
        }
    );
}

#[test]
fn 等齐_都说过话才交() {
    let direct = HashSet::new();
    let options = WaitOptions {
        wait_for: vec![ADA.to_owned(), NOVA.to_owned()],
        ..WaitOptions::default()
    };
    let mut messages = vec![(human(ADA, "我觉得行"), 0)];
    assert_eq!(
        decide(
            &arrivals(&messages),
            &options,
            context(&direct),
            20,
            10_000,
            None
        ),
        WaitDecision::Wait {
            recheck_at_ms: None
        },
        "只有 Ada 回了"
    );

    messages.push((agent(NOVA, "我也同意"), 12_000));
    assert_eq!(
        decide(
            &arrivals(&messages),
            &options,
            context(&direct),
            20,
            12_500,
            None
        ),
        WaitDecision::Wait {
            recheck_at_ms: Some(17_000)
        }
    );
    let delivery = delivered(decide(
        &arrivals(&messages),
        &options,
        context(&direct),
        20,
        17_000,
        None,
    ));
    assert_eq!(delivery.wake.reason, WakeReason::AllReplied);
    assert_eq!(delivery.picks, [0, 1]);
    assert!(delivery.wake.missing.is_empty());
}

#[test]
fn 等齐_等满时间还没齐_先给已经到的并说明谁还没说话() {
    let direct = HashSet::new();
    let options = WaitOptions {
        wait_for: vec![ADA.to_owned(), NOVA.to_owned(), ADA.to_owned()],
        ..WaitOptions::default()
    };
    let messages = [(human(ADA, "我觉得行"), 0)];
    let delivery = delivered(decide(
        &arrivals(&messages),
        &options,
        context(&direct),
        20,
        20_000,
        Some(20_000),
    ));
    assert_eq!(delivery.wake.reason, WakeReason::Timeout);
    assert_eq!(delivery.picks, [0]);
    assert_eq!(delivery.wake.event_ids, ["$我觉得行:matrix.test"]);
    assert_eq!(delivery.wake.missing, [NOVA]);
}

#[test]
fn 定时看一眼_没叫醒它的消息攒够时间就交() {
    let direct = HashSet::new();
    let options = WaitOptions {
        digest: Some(Duration::from_mins(30)),
        ..WaitOptions::default()
    };
    let messages = [(agent(NOVA, "a"), 0), (agent(NOVA, "b"), 60_000)];
    assert_eq!(
        decide(
            &arrivals(&messages),
            &options,
            context(&direct),
            20,
            61_000,
            None
        ),
        WaitDecision::Wait {
            recheck_at_ms: Some(1_800_000)
        }
    );
    let delivery = delivered(decide(
        &arrivals(&messages),
        &options,
        context(&direct),
        20,
        1_800_000,
        None,
    ));
    assert_eq!(delivery.wake.reason, WakeReason::Digest);
    assert_eq!(delivery.picks, [0, 1]);
    assert!(delivery.wake.event_ids.is_empty());
}

#[test]
fn 新消息太多时_叫醒它的一定给_剩下的名额给最新的() {
    let direct = HashSet::new();
    let options = WaitOptions {
        settle: Duration::ZERO,
        ..WaitOptions::default()
    };
    let messages: Vec<_> = (0..30)
        .map(|index| {
            let text = format!("第 {index} 条");
            let preview = if index == 3 {
                mentioning(agent(NOVA, &text), &[ME])
            } else {
                agent(NOVA, &text)
            };
            (preview, 0)
        })
        .collect();
    let delivery = delivered(decide(
        &arrivals(&messages),
        &options,
        context(&direct),
        5,
        1_000,
        None,
    ));
    assert_eq!(delivery.picks, [3, 26, 27, 28, 29]);
    assert_eq!(delivery.skipped, 25);
    assert_eq!(delivery.remaining, 0);
    assert_eq!(delivery.wake.event_ids, ["$第 3 条:matrix.test"]);

    // 叫醒它的比名额还多：先给最早的，剩下的下次给。
    let busy: Vec<_> = (0..10)
        .map(|index| (human(ADA, &format!("第 {index} 句")), 0))
        .collect();
    let delivery = delivered(decide(
        &arrivals(&busy),
        &options,
        context(&direct),
        4,
        1_000,
        None,
    ));
    assert_eq!(delivery.picks, [0, 1, 2, 3]);
    assert_eq!(delivery.skipped, 0);
    assert_eq!(delivery.remaining, 6);
}

#[test]
fn 只听一个房间_自己发的和别的房间的都不交() {
    let direct = HashSet::new();
    let options = WaitOptions {
        room_id: Some(ROOM.to_owned()),
        settle: Duration::ZERO,
        ..WaitOptions::default()
    };
    let messages = [
        (agent(ME, "我说的"), 0),
        (in_room(human(ADA, "别处"), OTHER_ROOM), 0),
        (human(BOB, "这里"), 0),
    ];
    let delivery = delivered(decide(
        &arrivals(&messages),
        &options,
        context(&direct),
        20,
        1_000,
        None,
    ));
    assert_eq!(delivery.picks, [2]);
    assert_eq!((delivery.skipped, delivery.remaining), (0, 0));
}

#[test]
fn 只看一眼_有什么给什么() {
    let direct = HashSet::new();
    let messages = [(agent(NOVA, "没点你"), 0), (agent(ME, "我说的"), 0)];
    let delivery = delivered(decide(
        &arrivals(&messages),
        &WaitOptions::peek(None),
        context(&direct),
        20,
        0,
        Some(0),
    ));
    assert_eq!(delivery.picks, [0], "自己发的照样不给");
    assert_eq!(delivery.wake.reason, WakeReason::Messages);
}

#[test]
fn 选项超出范围时指出是哪一项() {
    assert_eq!(WaitOptions::default().validate(), Ok(()));
    let invalid = [
        (
            WaitOptions {
                settle: Duration::from_secs(31),
                ..WaitOptions::default()
            },
            WaitOptionsField::Settle,
        ),
        (
            WaitOptions {
                digest: Some(Duration::from_secs(30)),
                ..WaitOptions::default()
            },
            WaitOptionsField::Digest,
        ),
        (
            WaitOptions {
                from: vec![ADA.to_owned(); MAX_PEOPLE + 1],
                ..WaitOptions::default()
            },
            WaitOptionsField::From,
        ),
        (
            WaitOptions {
                wait_for: vec![String::new()],
                ..WaitOptions::default()
            },
            WaitOptionsField::WaitFor,
        ),
    ];
    for (options, field) in invalid {
        assert_eq!(options.validate(), Err(field));
    }
}

#[test]
fn 等我上一条点到的人() {
    let asked = mentioning(agent(ME, "Ada、Nova 你们怎么看"), &[ADA, NOVA]);
    assert_eq!(mentioned_people(Some(&asked)), [ADA, NOVA]);
    assert!(mentioned_people(None).is_empty());
}

#[test]
fn 回答里的_wake_省掉空的列表() {
    let wake = IpcWake {
        reason: WakeReason::AllReplied,
        event_ids: vec!["$a:matrix.test".to_owned()],
        missing: Vec::new(),
    };
    let value = serde_json::to_value(&wake).expect("可编码");
    assert_eq!(
        value,
        serde_json::json!({"reason": "all_replied", "eventIds": ["$a:matrix.test"]})
    );
    assert_eq!(
        serde_json::from_value::<IpcWake>(value).expect("可解码"),
        wake
    );
    assert_eq!(
        serde_json::to_value(IpcWake::empty(WakeReason::Superseded)).expect("可编码"),
        serde_json::json!({"reason": "superseded"})
    );
}
