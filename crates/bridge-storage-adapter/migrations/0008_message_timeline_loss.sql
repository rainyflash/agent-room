-- 时间线上补不回来的一段（specs/agent-reading/design.md 第 4 步的 gaps）。
-- 同步时记下缺口的两头：之前最后一条、这次同步来的第一条消息。
ALTER TABLE message_timeline_gap ADD COLUMN after_event_id TEXT;
ALTER TABLE message_timeline_gap ADD COLUMN before_event_id TEXT;

-- 往回补没接上、或者根本没法往回补的一段：before_event_id 这条前面少了消息。
CREATE TABLE message_timeline_loss (
    room_id TEXT NOT NULL,
    before_event_id TEXT NOT NULL,
    after_event_id TEXT,
    reason TEXT NOT NULL,
    PRIMARY KEY (room_id, before_event_id)
);
