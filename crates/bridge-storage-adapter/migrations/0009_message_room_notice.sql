-- 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：同步里看到的最新一版，和交给这个
-- Agent 的是哪一版。进房间时交现在的，之后改了，下一次等消息交一次。
-- 撤掉了的公告正文是 NULL，时间照样记下：撤掉也是一次改动。
ALTER TABLE message_room_state ADD COLUMN notice_text TEXT;
ALTER TABLE message_room_state ADD COLUMN notice_updated_by TEXT;
ALTER TABLE message_room_state ADD COLUMN notice_updated_at_unix_ms INTEGER;
ALTER TABLE message_room_state ADD COLUMN notice_given_at_unix_ms INTEGER;
