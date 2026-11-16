//! 房间的置顶（`specs/redesign/brief-and-pins.md`“置顶”）：把房间里已经有的一句置顶，不另写一段文字。
//! 一间房最多 [`MAX_ROOM_PINS`] 条，新的在最前面；满了要先取消一条。只记是哪一句、谁、什么时候，
//! 不记那句写了什么：加密房间里那句照旧是加密的。

use crate::{DomainError, DomainResult, time::UtcMillis};

/// 一间房最多置顶几条。
pub const MAX_ROOM_PINS: usize = 5;

/// Matrix 事件 ID 最长多少字节（Matrix 规范的上限）。
const MAX_EVENT_ID_BYTES: usize = 255;

/// 置顶的那一句在 Matrix 上的事件 ID：`$` 开头，最长 255 字节，不带空白和控制字符。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RoomPinEventId(String);

impl RoomPinEventId {
    /// 解析网页、Agent 给的事件 ID。
    ///
    /// # Errors
    ///
    /// 不是 `$` 开头、太长，或带着空白、控制字符时返回错误。
    pub fn parse(input: &str) -> DomainResult<Self> {
        let valid = input.len() > 1
            && input.len() <= MAX_EVENT_ID_BYTES
            && input.starts_with('$')
            && !input
                .chars()
                .any(|character| character.is_whitespace() || character.is_control());
        if !valid {
            return Err(DomainError::Validation {
                field: "room_pin.event_id",
                reason: "要置顶的是一条 Matrix 事件：$ 开头，最长 255 字节，不带空白",
            });
        }
        Ok(Self(input.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 置顶的一句：哪一句、谁置顶的（Matrix 上的用户 ID）、什么时候。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomPin {
    pub event_id: RoomPinEventId,
    pub pinned_by: String,
    pub pinned_at: UtcMillis,
}

/// 一间房此刻的置顶，新的在最前面。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoomPins(Vec<RoomPin>);

impl RoomPins {
    /// 从房间状态里读出来的列表。同一句出现两次只留前面那条；超过上限的照旧留着（只是不能再置顶新的）。
    #[must_use]
    pub fn new(entries: Vec<RoomPin>) -> Self {
        let mut kept: Vec<RoomPin> = Vec::with_capacity(entries.len());
        for entry in entries {
            if !kept.iter().any(|pin| pin.event_id == entry.event_id) {
                kept.push(entry);
            }
        }
        Self(kept)
    }

    #[must_use]
    pub fn entries(&self) -> &[RoomPin] {
        &self.0
    }

    #[must_use]
    pub fn contains(&self, event_id: &RoomPinEventId) -> bool {
        self.0.iter().any(|pin| &pin.event_id == event_id)
    }

    /// 置顶一句，放在最前面。已经置顶着的不动，返回 `false`。
    ///
    /// # Errors
    ///
    /// 已经有 [`MAX_ROOM_PINS`] 条时返回错误：要先取消一条。
    pub fn pin(&mut self, pin: RoomPin) -> DomainResult<bool> {
        if self.contains(&pin.event_id) {
            return Ok(false);
        }
        if self.0.len() >= MAX_ROOM_PINS {
            return Err(DomainError::CapacityExceeded {
                capacity: u16::try_from(MAX_ROOM_PINS).unwrap_or(u16::MAX),
            });
        }
        self.0.insert(0, pin);
        Ok(true)
    }

    /// 取消置顶一句。本来就没置顶的不动，返回 `false`；那句删了也能取消。
    pub fn unpin(&mut self, event_id: &RoomPinEventId) -> bool {
        let before = self.0.len();
        self.0.retain(|pin| &pin.event_id != event_id);
        self.0.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> RoomPinEventId {
        RoomPinEventId::parse(value).unwrap()
    }

    fn pin(value: &str, at: i64) -> RoomPin {
        RoomPin {
            event_id: id(value),
            pinned_by: "@owner:matrix.test".to_owned(),
            pinned_at: UtcMillis::new(at).unwrap(),
        }
    }

    #[test]
    fn 事件_id_要像_matrix_的() {
        assert_eq!(id("$abc:matrix.test").as_str(), "$abc:matrix.test");
        assert!(RoomPinEventId::parse("$Rqnc-F-dvnEYJTyHq_iKxU2bZ1CI92-kuZq3a5lr5Zg").is_ok());
        for bad in ["", "$", "abc", "!room:matrix.test", "$a b", "$a\nb"] {
            assert!(RoomPinEventId::parse(bad).is_err(), "{bad:?}");
        }
        assert!(RoomPinEventId::parse(&format!("${}", "a".repeat(254))).is_ok());
        assert!(RoomPinEventId::parse(&format!("${}", "a".repeat(255))).is_err());
    }

    #[test]
    fn 新的在最前面_重复置顶不动_满了不收() {
        let mut pins = RoomPins::default();
        assert!(pins.pin(pin("$1", 1)).unwrap());
        assert!(pins.pin(pin("$2", 2)).unwrap());
        assert!(!pins.pin(pin("$1", 3)).unwrap());
        let order: Vec<_> = pins.entries().iter().map(|p| p.event_id.as_str()).collect();
        assert_eq!(order, ["$2", "$1"]);
        assert_eq!(pins.entries()[1].pinned_at.value(), 1);

        for n in 3..=5 {
            pins.pin(pin(&format!("${n}"), n)).unwrap();
        }
        assert_eq!(pins.entries().len(), MAX_ROOM_PINS);
        assert_eq!(
            pins.pin(pin("$6", 6)),
            Err(DomainError::CapacityExceeded { capacity: 5 })
        );
        assert!(!pins.pin(pin("$3", 7)).unwrap(), "已经置顶的照旧不动");
    }

    #[test]
    fn 取消置顶_没置顶的不动() {
        let mut pins = RoomPins::new(vec![pin("$2", 2), pin("$1", 1)]);
        assert!(pins.unpin(&id("$1")));
        assert!(!pins.unpin(&id("$1")));
        assert_eq!(pins.entries().len(), 1);
    }

    #[test]
    fn 读出来的列表去掉重复的() {
        let pins = RoomPins::new(vec![pin("$2", 2), pin("$1", 1), pin("$2", 3)]);
        let order: Vec<_> = pins.entries().iter().map(|p| p.pinned_at.value()).collect();
        assert_eq!(order, [2, 1]);
    }
}
