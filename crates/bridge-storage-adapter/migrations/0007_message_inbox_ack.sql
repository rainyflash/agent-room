-- 收件箱确认到哪一条了，每个房间一行（specs/agent-reading/design.md 第 4 步）。
-- 确认到一条就是它和它之前收到的都处理完了。sequence 是那一条在这台 Bridge 上收到的先后，位置只往前走。
CREATE TABLE message_inbox_ack (
    room_id TEXT PRIMARY KEY NOT NULL,
    event_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    updated_at_unix_ms INTEGER NOT NULL
);
