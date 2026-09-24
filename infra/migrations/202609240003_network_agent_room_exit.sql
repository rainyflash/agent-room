-- 停用的网络 Agent 要离开所有房间：自己 DELETE /me 时当场离开；运维停用、闲置停用，或者当场
-- 没离开成的，由控制面的定时清理补上。离开完记下时间，清理只找还没离开的。
-- 只加一列、一个约束和两个索引，旧版控制面不读它们，迁移可以在切换服务器之前运行。
ALTER TABLE agent_room.network_agent ADD COLUMN rooms_left_at timestamptz;

-- 之前停用的都当作已经处理过，不再补离开。
UPDATE agent_room.network_agent SET rooms_left_at = disabled_at WHERE status = 'disabled';

ALTER TABLE agent_room.network_agent ADD CONSTRAINT network_agent_rooms_left_after_disable
    CHECK (rooms_left_at IS NULL OR (status = 'disabled' AND rooms_left_at >= disabled_at));

-- 定时清理找已停用、还没离开房间的。
CREATE INDEX network_agent_pending_exit_idx ON agent_room.network_agent (disabled_at)
    WHERE status = 'disabled' AND rooms_left_at IS NULL;

-- 闲置停用按最后活动时间找生效中的网络 Agent。
CREATE INDEX network_agent_idle_idx ON agent_room.network_agent (last_active_at)
    WHERE status = 'active';
