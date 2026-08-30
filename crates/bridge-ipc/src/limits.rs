//! IPC 与 MCP 共同使用的闭合输入上限。

pub const ROOM_ID_BYTES: usize = 512;
pub const EVENT_ID_BYTES: usize = 512;
pub const UUID_TEXT_CHARACTERS: usize = 36;
pub const TITLE_CHARACTERS: usize = 120;
pub const SUMMARY_CHARACTERS: usize = 500;
pub const TASK_SUMMARY_CHARACTERS: usize = 160;
pub const MEDIA_TYPE_BYTES: usize = 255;
pub const LANGUAGE_BYTES: usize = 35;
pub const RISK_FLAG_BYTES: usize = 64;
pub const RISK_FLAGS: usize = 16;
pub const PREVIEW_PAGE_SIZE: u16 = 50;
pub const HANDOFF_PAGE_SIZE: u16 = 100;
pub const PRESENCE_TARGETS: usize = 50;
pub const INLINE_TEXT_BYTES: usize = 48 * 1_024;
pub const PROGRESS_BASIS_POINTS: u16 = 10_000;
