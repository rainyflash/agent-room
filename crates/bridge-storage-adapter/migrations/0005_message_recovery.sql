-- 解不开的加密事件按本机观察顺序预留一个序号，并记下它用的会话和发送设备。
-- 找回房间密钥后凭会话找出这些事件重读，读出来的消息写在预留的序号上，正好是原来的位置。
ALTER TABLE message_sync_issue ADD COLUMN reserved_sequence INTEGER
    CHECK (reserved_sequence IS NULL OR reserved_sequence > 0);
ALTER TABLE message_sync_issue ADD COLUMN sender TEXT;
ALTER TABLE message_sync_issue ADD COLUMN sender_device TEXT;
ALTER TABLE message_sync_issue ADD COLUMN session_id TEXT;

CREATE INDEX message_sync_issue_reserved_idx
    ON message_sync_issue (room_id, reserved_sequence)
    WHERE reserved_sequence IS NOT NULL;

CREATE INDEX message_sync_issue_event_idx
    ON message_sync_issue (room_id, event_id);
