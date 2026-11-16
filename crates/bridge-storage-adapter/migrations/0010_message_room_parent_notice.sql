-- 子房间里父房间须知的副本（`specs/redesign/brief-and-pins.md`“子房间里父房间的须知也算”）：同步里
-- 看到的最新一版（父房间号、正文、改的人、改的时间），和交给这个 Agent 的是哪一版。和这一间自己的
-- 公告分开记、分开交：进房间时交现在的，之后改了，下一次等消息交一次。
-- 撤掉了的正文是 NULL，时间照样记下：撤掉也是一次改动。
ALTER TABLE message_room_state ADD COLUMN parent_notice_room_id TEXT;
ALTER TABLE message_room_state ADD COLUMN parent_notice_text TEXT;
ALTER TABLE message_room_state ADD COLUMN parent_notice_updated_by TEXT;
ALTER TABLE message_room_state ADD COLUMN parent_notice_updated_at_unix_ms INTEGER;
ALTER TABLE message_room_state ADD COLUMN parent_notice_given_at_unix_ms INTEGER;
