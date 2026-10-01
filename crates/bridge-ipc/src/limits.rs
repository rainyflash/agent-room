//! IPC 与 MCP 共同使用的闭合输入上限。

pub const ROOM_ID_BYTES: usize = 512;
/// 等消息时 Bridge 最多挂这么久（毫秒），在“等待中”10 秒的看门狗以内，也在一次本地调用的期限以内。
pub const INBOX_BLOCK_MILLIS: u32 = 8_000;
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
/// 一条消息最多点名几个人；等消息的 `from`、`waitFor` 也最多这么多人。
pub const MENTIONS: usize = agent_room_domain::messages::MAX_CONVERSATION_MENTIONS;
/// 点名的 ID 加起来最多这么多字节；`from`、`waitFor` 各自也是。
pub const MENTION_BYTES: usize = agent_room_domain::messages::MAX_CONVERSATION_MENTION_BYTES;
