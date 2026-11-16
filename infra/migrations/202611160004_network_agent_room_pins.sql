-- 房间的置顶交给网络 Agent（specs/redesign/brief-and-pins.md“Agent 读置顶”，4N-4c）：网关同步时把每个房间
-- 最新的一版置顶记在 network_agent_room 上（置顶的那几句的事件 ID、谁、什么时候置顶的，存成 JSON 数组，
-- 全取消了是空的；最近一次谁、什么时候改的），再记交给这个网络 Agent 的是哪一版。那几句写了什么不存在
-- 这里：交的时候照它的消息记录。置顶本来就是房间状态，服务器读得到，这里只是网关替网络 Agent 记的一份，
-- 它离开这个房间时那一行跟着删。
-- 只加列和约束，上一版控制面不读也不写这些列：迁移可以在切换服务器之前运行，回滚照常。
ALTER TABLE agent_room.network_agent_room
    ADD COLUMN pins jsonb,
    ADD COLUMN pins_updated_by text,
    ADD COLUMN pins_updated_at_ms bigint,
    ADD COLUMN pins_given_at_ms bigint;

ALTER TABLE agent_room.network_agent_room
    ADD CONSTRAINT network_agent_room_pins_version CHECK (
        (pins_updated_by IS NULL) = (pins_updated_at_ms IS NULL)
        AND (pins IS NULL OR (pins_updated_at_ms IS NOT NULL AND jsonb_typeof(pins) = 'array'))
        AND (pins_given_at_ms IS NULL OR pins_updated_at_ms IS NOT NULL)
    );
