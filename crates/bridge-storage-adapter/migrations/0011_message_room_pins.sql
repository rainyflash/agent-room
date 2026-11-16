-- 房间的置顶（`specs/redesign/brief-and-pins.md`“Agent 读置顶”）：同步里看到的最新一版（置顶的那几句的
-- 事件 ID、谁、什么时候置顶的，存成 JSON；最近一次谁、什么时候改的），和交给这个 Agent 的是哪一版。
-- 进房间时交现在的，之后改了，下一次等消息交一次整份。全取消了 `pins_json` 是 NULL，时间照样记下。
-- 那几句写了什么不存在这里：交的时候照消息库里已经有的那一条。
ALTER TABLE message_room_state ADD COLUMN pins_json TEXT;
ALTER TABLE message_room_state ADD COLUMN pins_updated_by TEXT;
ALTER TABLE message_room_state ADD COLUMN pins_updated_at_unix_ms INTEGER;
ALTER TABLE message_room_state ADD COLUMN pins_given_at_unix_ms INTEGER;
