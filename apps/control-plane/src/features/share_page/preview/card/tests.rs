//! 预览图上的字：换行、省略、转义、两种语言、仓库和下面那一行。字宽用一个好算的假字体（中文一个 em，
//! 英文半个），真字体的在 `preview/tests.rs`。

use super::*;

/// 换行的测试都按 64 像素、最多两行量。
const TITLE_SIZE: f32 = 64.0;
const TITLE_LINES: usize = 2;

/// 中日韩的字一个 em，空格四分之一，别的 ASCII 半个，“…”一个；表情没有。
struct EvenMeasure;

impl Measure for EvenMeasure {
    fn advance(&self, character: char) -> Option<f32> {
        match character {
            ' ' => Some(0.25),
            '…' => Some(1.0),
            character if character.is_ascii() => Some(0.5),
            character if wide(character) => Some(1.0),
            _ => None,
        }
    }
}

fn room(name: &str, language: Language) -> PreviewCard {
    PreviewCard {
        language,
        name: name.to_owned(),
        repository: None,
        address: "agent-room.test/r/aB3xYz".to_owned(),
        kind: CardKind::Room {
            online: Some((12, false)),
            people: 3,
            agents: 5,
        },
    }
}

fn texts(svg: &str) -> Vec<String> {
    svg.split("<text ")
        .skip(1)
        .map(|chunk| {
            let body = &chunk[chunk.find('>').unwrap() + 1..chunk.find("</text>").unwrap()];
            body.replace("<tspan fill=\"#6b6b76\">", "")
                .replace("</tspan>", "")
        })
        .collect()
}

#[test]
fn 房间名和地址都转义_图里没有用户写的标签() {
    let mut card = room(
        "<script>alert(1)</script> & \"读书会\" 'x'",
        Language::Chinese,
    );
    card.address = "agent-room.test/gh/a&b/<c>".to_owned();
    let svg = svg(&card, &EvenMeasure);

    assert!(!svg.contains("<script"), "{svg}");
    assert!(!svg.contains("<c>"), "{svg}");
    assert!(svg.contains("&lt;script&gt;"), "{svg}");
    assert!(svg.contains("&amp;"), "{svg}");
    assert!(svg.contains("&quot;读书会&quot;"), "{svg}");
    assert!(svg.contains("&#39;x&#39;"), "{svg}");
    assert!(
        svg.contains("agent-room.test/gh/a&amp;b/&lt;c&gt;"),
        "{svg}"
    );
    assert!(
        svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1280\" height=\"640\"")
    );
}

#[test]
fn 长的中文名换两行_第二行省略() {
    // 64 像素一个字，一行 700 像素放得下 10 个。
    let name = "一二三四五六七八九十甲乙丙丁戊己庚辛壬癸子丑寅卯";
    let lines = wrap(name, TITLE_SIZE, BLOCK_WIDTH, TITLE_LINES, &EvenMeasure);
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["一二三四五六七八九十", "甲乙丙丁戊己庚辛壬…"]
    );
    for line in &lines {
        assert!(width(&line.text, TITLE_SIZE, &EvenMeasure) <= BLOCK_WIDTH);
    }
    // 一行放得下的不换行，也不加省略号。
    let short = wrap("读书会", TITLE_SIZE, BLOCK_WIDTH, TITLE_LINES, &EvenMeasure);
    assert_eq!(short.len(), 1);
    assert_eq!(short[0].text, "读书会");
    // “，”这类字不放到行首：连前面那个字一起挪到下一行。
    let comma = wrap(
        "一二三四五六七八九十，甲",
        TITLE_SIZE,
        BLOCK_WIDTH,
        TITLE_LINES,
        &EvenMeasure,
    );
    assert_eq!(comma[0].text, "一二三四五六七八九");
    assert_eq!(comma[1].text, "十，甲");
}

// 字号是写死的那几个整数，不是算出来的，可以直接比。
#[allow(clippy::float_cmp)]
#[test]
fn 房间名一行放得下就一行_放不下先缩小_再放不下两行() {
    let size = |name: &str| {
        let (size, lines) = title_lines(name, false, &EvenMeasure);
        (
            size,
            lines.into_iter().map(|line| line.text).collect::<Vec<_>>(),
        )
    };
    assert_eq!(size("读书会"), (64.0, vec!["读书会".to_owned()]));
    // 64 像素放 10 个字，56 像素放 12 个。
    assert_eq!(size("一二三四五六七八九十甲").0, 56.0);
    assert_eq!(size("一二三四五六七八九十甲乙丙丁").0, 48.0);
    let (two, lines) = size("一二三四五六七八九十甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳午未申酉");
    assert_eq!(two, 56.0);
    assert_eq!(
        lines,
        ["一二三四五六七八九十甲乙", "丙丁戊己庚辛壬癸子丑寅…"]
    );
    // 两行的 SVG：两行字，第二行比第一行低一行。
    let svg = svg(
        &room(
            "一二三四五六七八九十甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳午未申酉",
            Language::Chinese,
        ),
        &EvenMeasure,
    );
    assert!(svg.contains("y=\"250\" font-size=\"56\""), "{svg}");
    assert!(svg.contains("y=\"317\" font-size=\"56\""), "{svg}");
}

#[test]
fn 长的英文名在空格处换行_第二行省略() {
    // 32 像素一个字母，一行放得下 21 个。
    let name = "Weekly reading circle for people who love long books";
    let lines = wrap(name, TITLE_SIZE, BLOCK_WIDTH, TITLE_LINES, &EvenMeasure);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].text, "Weekly reading circle");
    assert!(lines[1].text.starts_with("for people who"), "{lines:?}");
    assert!(lines[1].text.ends_with('…'), "{lines:?}");
    assert!(!lines[1].text.contains(" …"), "省略号前不留空格：{lines:?}");
    for line in &lines {
        assert!(width(&line.text, TITLE_SIZE, &EvenMeasure) <= BLOCK_WIDTH);
    }
    // 一个词比一行还长：从中间断。
    let word = "a".repeat(30);
    let broken = wrap(&word, TITLE_SIZE, BLOCK_WIDTH, TITLE_LINES, &EvenMeasure);
    assert_eq!(broken[0].text, "a".repeat(21));
    assert_eq!(broken[1].text, "a".repeat(9));
    // `owner/repo` 在斜杠后面断。
    let repository = wrap(
        "rainyflash-labs/agent-room-plugins",
        TITLE_SIZE,
        BLOCK_WIDTH,
        TITLE_LINES,
        &EvenMeasure,
    );
    assert_eq!(repository[0].text, "rainyflash-labs/");
    assert_eq!(repository[1].text, "agent-room-plugins");
}

#[test]
fn 字体里没有的字去掉_空白压成一个() {
    assert_eq!(
        drawable("🚀  Rocket\n\tclub 🚀", &EvenMeasure),
        "Rocket club"
    );
    assert_eq!(drawable("读书\u{0}会", &EvenMeasure), "读书会");
    // 全是画不出来的：照原样留着，总比空着好。
    assert_eq!(drawable("🚀🚀", &EvenMeasure), "🚀🚀");
}

#[test]
fn 中文的那一行_在线问得到问不到() {
    let card = room("读书会", Language::Chinese);
    assert_eq!(summary(&card), "现在 12 个在线 · 3 人 · 5 个 Agent");

    let mut at_least = card.clone();
    at_least.kind = CardKind::Room {
        online: Some((200, true)),
        people: 250,
        agents: 1,
    };
    assert_eq!(
        summary(&at_least),
        "现在至少 200 个在线 · 250 人 · 1 个 Agent"
    );

    let mut unknown = card;
    unknown.kind = CardKind::Room {
        online: None,
        people: 3,
        agents: 5,
    };
    assert_eq!(summary(&unknown), "3 人 · 5 个 Agent");
    assert!(texts(&svg(&unknown, &EvenMeasure)).contains(&"3 人 · 5 个 Agent".to_owned()));
}

#[test]
fn 英文的那一行_单复数() {
    let card = room("Book club", Language::English);
    assert_eq!(summary(&card), "12 online · 3 people · 5 agents");

    let mut one = card.clone();
    one.kind = CardKind::Room {
        online: Some((200, true)),
        people: 1,
        agents: 1,
    };
    assert_eq!(summary(&one), "200+ online · 1 person · 1 agent");

    let mut unknown = card;
    unknown.kind = CardKind::Room {
        online: None,
        people: 0,
        agents: 2,
    };
    assert_eq!(summary(&unknown), "0 people · 2 agents");
    let svg = svg(&unknown, &EvenMeasure);
    assert!(texts(&svg).contains(&"Public room".to_owned()));
    assert!(!svg.contains("公开房间"));
}

#[test]
fn 一段对话写几句_左上写是一段对话() {
    let mut card = room("读书会", Language::Chinese);
    card.kind = CardKind::Excerpt { messages: 6 };
    assert_eq!(summary(&card), "一段对话 · 6 句");
    let shown = texts(&svg(&card, &EvenMeasure));
    assert!(
        shown.contains(&"公开房间里的一段对话".to_owned()),
        "{shown:?}"
    );
    assert!(!shown.iter().any(|text| text.contains("在线")), "{shown:?}");

    card.language = Language::English;
    card.kind = CardKind::Excerpt { messages: 1 };
    assert_eq!(summary(&card), "A conversation · 1 message");
    card.kind = CardKind::Excerpt { messages: 6 };
    assert_eq!(summary(&card), "A conversation · 6 messages");
    let shown = texts(&svg(&card, &EvenMeasure));
    assert!(
        shown.contains(&"A conversation from a public room".to_owned()),
        "{shown:?}"
    );
}

#[test]
fn 名字就是仓库时_owner_写浅一点_勾跟在名字后面() {
    let mut card = room("rainyflash/agent-room", Language::English);
    card.repository = Some("RainyFlash/Agent-Room".to_owned());
    card.address = "agent-room.test/gh/RainyFlash/Agent-Room".to_owned();
    let svg = svg(&card, &EvenMeasure);

    assert!(
        svg.contains("<tspan fill=\"#6b6b76\">rainyflash/</tspan>agent-room</text>"),
        "{svg}"
    );
    assert_eq!(
        svg.matches("<circle cx=\"12\" cy=\"12\" r=\"8.5\"/>")
            .count(),
        1
    );
    // 不另起一行再写一遍仓库。
    let shown = texts(&svg);
    assert_eq!(
        shown
            .iter()
            .filter(|text| text.eq_ignore_ascii_case("rainyflash/agent-room"))
            .count(),
        1,
        "{shown:?}"
    );
    assert!(shown.contains(&"agent-room.test/gh/RainyFlash/Agent-Room".to_owned()));
}

#[test]
fn 名字不是仓库时_下面另起一行写仓库和勾() {
    let mut card = room("读书会", Language::Chinese);
    card.repository = Some("octo/hello".to_owned());
    let svg = svg(&card, &EvenMeasure);
    let shown = texts(&svg);

    assert!(shown.contains(&"读书会".to_owned()), "{shown:?}");
    assert!(shown.contains(&"octo/hello".to_owned()), "{shown:?}");
    assert!(!svg.contains("<tspan"), "房间名不写浅色");
    assert_eq!(
        svg.matches("<circle cx=\"12\" cy=\"12\" r=\"8.5\"/>")
            .count(),
        1
    );

    let plain = svg_without_repository();
    assert!(!plain.contains("r=\"8.5\""), "没绑仓库没有勾");
}

fn svg_without_repository() -> String {
    svg(&room("读书会", Language::Chinese), &EvenMeasure)
}

#[test]
fn 机器人底下的三个圆都有颜色_别的地方只有黑白灰() {
    let svg = svg(&room("读书会", Language::Chinese), &EvenMeasure);
    for robot in &ROBOTS {
        assert!(svg.contains(&format!("fill=\"{}\"", robot.fill)), "{svg}");
    }
    let colors: Vec<&str> = svg
        .match_indices("fill=\"#")
        .map(|(index, _)| &svg[index + 6..index + 13])
        .collect();
    for color in colors {
        assert!(
            [INK, MUTED, PAPER].contains(&color) || ROBOTS.iter().any(|robot| robot.fill == color),
            "{color}"
        );
    }
}

#[test]
fn 太长的地址和那一行也省略() {
    let long = format!("agent-room.test/gh/{}/{}", "o".repeat(39), "r".repeat(100));
    let shown = ellipsize(&long, FOOTER_SIZE, FOOTER_WIDTH, &EvenMeasure);
    assert!(shown.ends_with('…'));
    assert!(width(&shown, FOOTER_SIZE, &EvenMeasure) <= FOOTER_WIDTH);
    assert_eq!(
        ellipsize("短的", FOOTER_SIZE, FOOTER_WIDTH, &EvenMeasure),
        "短的"
    );
}
