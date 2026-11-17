//! 预览图上画什么、画在哪：拼一张 1280×640 的 SVG（白底黑字，颜色只给右边的机器人，机器人本身由
//! [`super::draw`] 贴上去）。
//!
//! 图上的字只有这几样：左上“Agent Room”，大字的房间名（最多两行，长了省略），绑了仓库的写 `owner/repo`
//! 和那个勾，下面一行几个在线、几人、几个 Agent（一段对话写“一段对话 · 6 句”），左下这间房的地址。没有
//! 一句房间里说的话。房间名是用户写的，进 SVG 之前一律转义。
//!
//! 换行和省略按字体真的字宽量（[`Measure`]），中文、英文一样。房间名一行放得下就用一行（64 像素放不下先
//! 缩到 56、48），还放不下就 56 像素两行，第二行放不下就省略。

use std::fmt::Write as _;

use super::super::{Language, escape};

pub(in crate::features::share_page) const WIDTH: u32 = 1280;
pub(in crate::features::share_page) const HEIGHT: u32 = 640;

const INK: &str = "#18181b";
const MUTED: &str = "#6b6b76";
const PAPER: &str = "#ffffff";

/// 字从这里起。
const LEFT: f32 = 80.0;
/// 房间名、仓库、人数那一块最宽多少：右边留给机器人。
const BLOCK_WIDTH: f32 = 700.0;
/// 左下的地址最宽多少。
const FOOTER_WIDTH: f32 = 1_120.0;

const HEADER_SIZE: f32 = 28.0;
const HEADER_NOTE_SIZE: f32 = 22.0;
const HEADER_BASELINE: f32 = 96.0;
/// 房间名一行时依次试的字号。
const ONE_LINE_SIZES: [f32; 3] = [64.0, 56.0, 48.0];
/// 一行放不下时两行的字号。
const TWO_LINE_SIZE: f32 = 56.0;
const TITLE_TOP: f32 = 196.0;
/// 跟在房间名后面的勾、和下面那一行前面的勾离字多远。
const MARK_GAP: f32 = 14.0;
const LINE_SIZE: f32 = 30.0;
const FOOTER_SIZE: f32 = 22.0;
const FOOTER_BASELINE: f32 = 584.0;
const ELLIPSIS: char = '…';

/// 机器人：底下一个浅色的圆，圆心、半径、圆的颜色，图片在圆里占多宽（图片本身由 `draw` 贴）。
pub(in crate::features::share_page) struct RobotSpot {
    pub(in crate::features::share_page) center: (f32, f32),
    pub(in crate::features::share_page) radius: f32,
    pub(in crate::features::share_page) fill: &'static str,
    pub(in crate::features::share_page) image: f32,
}

/// 三个机器人：方的（青）、圆的（橙）、胶囊的（黄），和 `draw` 里的三张图同一个顺序。位置照设计稿 10。
pub(in crate::features::share_page) const ROBOTS: [RobotSpot; 3] = [
    RobotSpot {
        center: (1_025.0, 245.0),
        radius: 95.0,
        fill: "#e2f3ef",
        image: 141.0,
    },
    RobotSpot {
        center: (885.0, 395.0),
        radius: 75.0,
        fill: "#fde9e5",
        image: 111.0,
    },
    RobotSpot {
        center: (1_100.0, 410.0),
        radius: 70.0,
        fill: "#fff2cc",
        image: 104.0,
    },
];

/// 一张图要画的全部东西。只来自目录的详情（分享页上本来就公开的）和仓库绑定。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(in crate::features::share_page) struct PreviewCard {
    pub(in crate::features::share_page) language: Language,
    pub(in crate::features::share_page) name: String,
    /// 绑着的仓库，`owner/repo`。
    pub(in crate::features::share_page) repository: Option<String>,
    /// 左下写的地址，不带 `https://`。
    pub(in crate::features::share_page) address: String,
    pub(in crate::features::share_page) kind: CardKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::features::share_page) enum CardKind {
    Room {
        /// 现在几个在线、是不是“至少”；问不到是 `None`，那一段不写。
        online: Option<(u32, bool)>,
        people: u32,
        agents: u32,
    },
    Excerpt {
        messages: u32,
    },
}

/// 量字：一个字在 1 像素一个 em 时多宽。字体里没有这个字是 `None`。
pub(in crate::features::share_page) trait Measure {
    fn advance(&self, character: char) -> Option<f32>;
}

/// 拼好的 SVG。
pub(in crate::features::share_page) fn svg(card: &PreviewCard, measure: &dyn Measure) -> String {
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{HEIGHT}\" \
         viewBox=\"0 0 {WIDTH} {HEIGHT}\"><rect width=\"{WIDTH}\" height=\"{HEIGHT}\" \
         fill=\"{PAPER}\"/><g font-weight=\"700\">"
    );
    svg.push_str(&header(card, measure));
    svg.push_str(&body(card, measure));
    let address = ellipsize(
        &drawable(&card.address, measure),
        FOOTER_SIZE,
        FOOTER_WIDTH,
        measure,
    );
    svg.push_str(&text_line(
        LEFT,
        FOOTER_BASELINE,
        FOOTER_SIZE,
        MUTED,
        &address,
    ));
    svg.push_str("</g>");
    for robot in &ROBOTS {
        let _ = write!(
            svg,
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{}\"/>",
            robot.center.0, robot.center.1, robot.radius, robot.fill
        );
    }
    svg.push_str("</svg>");
    svg
}

fn header(card: &PreviewCard, measure: &dyn Measure) -> String {
    let note = match (card.kind, card.language) {
        (CardKind::Room { .. }, Language::Chinese) => "公开房间",
        (CardKind::Room { .. }, Language::English) => "Public room",
        (CardKind::Excerpt { .. }, Language::Chinese) => "公开房间里的一段对话",
        (CardKind::Excerpt { .. }, Language::English) => "A conversation from a public room",
    };
    let brand = "Agent Room";
    let note_x = LEFT + width(brand, HEADER_SIZE, measure) + MARK_GAP;
    format!(
        "{}{}",
        text_line(LEFT, HEADER_BASELINE, HEADER_SIZE, INK, brand),
        text_line(note_x, HEADER_BASELINE, HEADER_NOTE_SIZE, MUTED, note),
    )
}

/// 房间名、仓库、下面那一行。
fn body(card: &PreviewCard, measure: &dyn Measure) -> String {
    let name = drawable(&card.name, measure);
    let repository = card
        .repository
        .as_deref()
        .map(|repository| drawable(repository, measure));
    // 项目房间的名字就是 `owner/repo`：不再另起一行写仓库，`owner/` 写浅一点，勾跟在名字后面。
    let named_after_repository = repository
        .as_deref()
        .is_some_and(|repository| repository.eq_ignore_ascii_case(&name));
    let (mut svg, mut cursor) = title(&name, named_after_repository, measure);
    if let Some(repository) = repository.filter(|_| !named_after_repository) {
        let baseline = cursor + 34.0;
        svg.push_str(&check_mark(LEFT, baseline - 0.35 * LINE_SIZE, LINE_SIZE));
        let x = LEFT + LINE_SIZE + MARK_GAP;
        let shown = ellipsize(
            &repository,
            LINE_SIZE,
            BLOCK_WIDTH - LINE_SIZE - MARK_GAP,
            measure,
        );
        svg.push_str(&text_line(x, baseline, LINE_SIZE, MUTED, &shown));
        cursor += 52.0;
    }
    let summary = ellipsize(&summary(card), LINE_SIZE, BLOCK_WIDTH, measure);
    svg.push_str(&text_line(LEFT, cursor + 40.0, LINE_SIZE, INK, &summary));
    svg
}

/// 大字的房间名，回 SVG 和下面接着从哪画。`marked` 时 `owner/` 写浅一点、后面跟着勾。
fn title(name: &str, marked: bool, measure: &dyn Measure) -> (String, f32) {
    let (size, lines) = title_lines(name, marked, measure);
    let muted_prefix = if marked {
        name.find('/')
            .map_or(0, |slash| name[..=slash].chars().count())
    } else {
        0
    };
    let line_height = size * 1.19;
    let mut svg = String::new();
    let mut baseline = TITLE_TOP + size * 0.97;
    for line in &lines {
        svg.push_str(&title_line(
            line,
            baseline,
            size,
            muted_prefix.saturating_sub(line.start),
        ));
        baseline += line_height;
    }
    if marked && let Some(last) = lines.last() {
        let x = LEFT + width(&last.text, size, measure) + MARK_GAP;
        svg.push_str(&check_mark(
            x,
            baseline - line_height - 0.35 * size,
            mark(size),
        ));
    }
    let rows = if lines.len() > 1 { 2.0 } else { 1.0 };
    (svg, TITLE_TOP + line_height * rows + 18.0)
}

/// 先试一行（字号从大到小），都放不下就两行。
fn title_lines(name: &str, marked: bool, measure: &dyn Measure) -> (f32, Vec<Line>) {
    for size in ONE_LINE_SIZES {
        let room = BLOCK_WIDTH - if marked { mark(size) + MARK_GAP } else { 0.0 };
        if width(name, size, measure) <= room {
            return (size, wrap(name, size, room, 1, measure));
        }
    }
    let room = BLOCK_WIDTH
        - if marked {
            mark(TWO_LINE_SIZE) + MARK_GAP
        } else {
            0.0
        };
    (TWO_LINE_SIZE, wrap(name, TWO_LINE_SIZE, room, 2, measure))
}

/// 跟在房间名后面的勾多大。
fn mark(size: f32) -> f32 {
    size * 0.75
}

/// 大字下面那一行：“现在 12 个在线 · 3 人 · 5 个 Agent”“一段对话 · 6 句”。
pub(in crate::features::share_page) fn summary(card: &PreviewCard) -> String {
    match (card.kind, card.language) {
        (
            CardKind::Room {
                online,
                people,
                agents,
            },
            Language::Chinese,
        ) => {
            let mut parts = Vec::new();
            if let Some((count, at_least)) = online {
                let now = if at_least { "现在至少" } else { "现在" };
                parts.push(format!("{now} {count} 个在线"));
            }
            parts.push(format!("{people} 人"));
            parts.push(format!("{agents} 个 Agent"));
            parts.join(" · ")
        }
        (
            CardKind::Room {
                online,
                people,
                agents,
            },
            Language::English,
        ) => {
            let mut parts = Vec::new();
            if let Some((count, at_least)) = online {
                let plus = if at_least { "+" } else { "" };
                parts.push(format!("{count}{plus} online"));
            }
            parts.push(plural(people, "person", "people"));
            parts.push(plural(agents, "agent", "agents"));
            parts.join(" · ")
        }
        (CardKind::Excerpt { messages }, Language::Chinese) => format!("一段对话 · {messages} 句"),
        (CardKind::Excerpt { messages }, Language::English) => format!(
            "A conversation · {}",
            plural(messages, "message", "messages")
        ),
    }
}

fn plural(count: u32, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// 一行字，`fill` 写在 `<text>` 上。位置取整到像素。
fn text_line(x: f32, baseline: f32, size: f32, fill: &str, text: &str) -> String {
    format!(
        "<text x=\"{}\" y=\"{}\" font-size=\"{size}\" fill=\"{fill}\">{}</text>",
        x.round(),
        baseline.round(),
        escape(text)
    )
}

/// 房间名的一行：前 `muted` 个字（项目房间的 `owner/`）写浅一点。
fn title_line(line: &Line, baseline: f32, size: f32, muted: usize) -> String {
    let split = line
        .text
        .char_indices()
        .nth(muted)
        .map_or(line.text.len(), |(index, _)| index);
    let (light, dark) = line.text.split_at(split);
    let mut svg = format!(
        "<text x=\"{LEFT}\" y=\"{}\" font-size=\"{size}\" fill=\"{INK}\">",
        baseline.round()
    );
    if !light.is_empty() {
        let _ = write!(svg, "<tspan fill=\"{MUTED}\">{}</tspan>", escape(light));
    }
    svg.push_str(&escape(dark));
    svg.push_str("</text>");
    svg
}

/// 那个勾：一个圈里一个对号，`size` 见方，左边在 `x`，竖着的中线在 `middle`。照房间页和设计稿的样子，
/// 24 见方画好再放大。
fn check_mark(x: f32, middle: f32, size: f32) -> String {
    let scale = size / 24.0;
    let (x, y) = (x.round(), (middle - size / 2.0).round());
    format!(
        "<g transform=\"translate({x} {y}) scale({scale})\" fill=\"none\" stroke=\"{MUTED}\" \
         stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\
         <circle cx=\"12\" cy=\"12\" r=\"8.5\"/><path d=\"M8.5 12.3l2.4 2.4 4.6-5\"/></g>"
    )
}

/// 换好的一行。`start` 是这一行的第一个字在原文里是第几个字，给浅色的前缀数位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::features::share_page) struct Line {
    pub(in crate::features::share_page) text: String,
    start: usize,
}

/// 量出来的宽（像素）。字体里没有的字按一个 em 算，反正画之前已经去掉了。
pub(in crate::features::share_page) fn width(text: &str, size: f32, measure: &dyn Measure) -> f32 {
    text.chars()
        .map(|character| measure.advance(character).unwrap_or(1.0))
        .sum::<f32>()
        * size
}

/// 压成一行、去掉控制字符和字体里没有的字（多半是表情，画出来是方框）。都去掉了就照原样留着。
pub(in crate::features::share_page) fn drawable(text: &str, measure: &dyn Measure) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let kept: String = line
        .chars()
        .filter(|character| {
            !character.is_control() && (*character == ' ' || measure.advance(*character).is_some())
        })
        .collect();
    let kept = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    if kept.is_empty() {
        line.chars()
            .filter(|character| !character.is_control())
            .collect()
    } else {
        kept
    }
}

/// 一行放不下就截短、加“…”。
pub(in crate::features::share_page) fn ellipsize(
    text: &str,
    size: f32,
    max_width: f32,
    measure: &dyn Measure,
) -> String {
    if width(text, size, measure) <= max_width {
        return text.to_owned();
    }
    let characters: Vec<char> = text.chars().collect();
    truncated(&characters, characters.len(), size, max_width, measure)
}

/// 前 `end` 个字往回截，截到加上“…”放得下。
fn truncated(
    characters: &[char],
    end: usize,
    size: f32,
    max_width: f32,
    measure: &dyn Measure,
) -> String {
    let ellipsis = measure.advance(ELLIPSIS).unwrap_or(1.0) * size;
    let mut end = end;
    while end > 0 {
        let head: String = characters[..end].iter().collect();
        if width(&head, size, measure) + ellipsis <= max_width {
            break;
        }
        end -= 1;
    }
    let mut head: String = characters[..end].iter().collect();
    head.truncate(head.trim_end().len());
    head.push(ELLIPSIS);
    head
}

/// 按宽度换行，最多 `max_lines` 行，最后一行放不下就省略。能断的地方：空格、`/`、`-`、`_` 后面，
/// 中日韩的字之间（不在“，。）”这类字前面断）；一个词比一行还长就从中间断。
pub(in crate::features::share_page) fn wrap(
    text: &str,
    size: f32,
    max_width: f32,
    max_lines: usize,
    measure: &dyn Measure,
) -> Vec<Line> {
    let characters: Vec<char> = text.chars().collect();
    let mut lines = Vec::new();
    let mut start = 0;
    while start < characters.len() && lines.len() < max_lines {
        while start < characters.len() && characters[start] == ' ' {
            start += 1;
        }
        let (end, last_break) = fill(&characters, start, size, max_width, measure);
        let (text, next) = if end == characters.len() {
            (characters[start..end].iter().collect::<String>(), end)
        } else if lines.len() + 1 == max_lines {
            let rest = &characters[start..];
            (
                truncated(rest, end - start, size, max_width, measure),
                characters.len(),
            )
        } else {
            let cut = last_break
                .filter(|cut| *cut > start)
                .unwrap_or_else(|| end.max(start + 1));
            let head: String = characters[start..cut].iter().collect();
            (head.trim_end().to_owned(), cut)
        };
        if !text.is_empty() {
            lines.push(Line { text, start });
        }
        start = next;
    }
    lines
}

/// 从 `start` 往后放，放到放不下为止：回答放到哪（不含）、最后一个能断开的位置。至少放一个字。
fn fill(
    characters: &[char],
    start: usize,
    size: f32,
    max_width: f32,
    measure: &dyn Measure,
) -> (usize, Option<usize>) {
    let mut used = 0.0;
    let mut end = start;
    let mut last_break = None;
    while end < characters.len() {
        let advance = measure.advance(characters[end]).unwrap_or(1.0) * size;
        if used + advance > max_width && end > start {
            break;
        }
        used += advance;
        end += 1;
        if end < characters.len() && breakable(characters[end - 1], characters[end]) {
            last_break = Some(end);
        }
    }
    (end, last_break)
}

/// `before` 和 `after` 之间能不能换行。
fn breakable(before: char, after: char) -> bool {
    if after == ' ' {
        return true;
    }
    if matches!(
        after,
        '，' | '。' | '、' | '！' | '？' | '：' | '；' | '）' | '】' | '」' | '』' | '》' | '…'
    ) {
        return false;
    }
    matches!(before, ' ' | '/' | '-' | '_') || wide(before) || wide(after)
}

/// 中日韩的字：字和字之间就能断行。
fn wide(character: char) -> bool {
    matches!(
        character,
        '\u{2E80}'..='\u{9FFF}'
            | '\u{AC00}'..='\u{D7AF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FF00}'..='\u{FFEF}'
    )
}

#[cfg(test)]
mod tests;
