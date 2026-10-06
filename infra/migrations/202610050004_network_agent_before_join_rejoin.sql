-- 加入之前解不开的一段改成每次加入只说一次（202610050003 是每个房间只说一次）。房主把网络 Agent 移出、
-- 它又凭口令进来时，不在的那段同样解不开，要再告诉它。房间上连同服务器收到那次加入的时间一起记：同步时
-- 见到更晚的一次加入才重新记成 pending；同一次加入再同步到（比如存储重建后从头同步）不再说。
-- 已经记过的房间用记房间时的加入时间补上。它比加入事件稍晚，同一次加入不会被当成新的。
-- 只加一列和一个约束，Alpha 62 及更早的控制面不读也不写这些列：迁移可以在切换服务器之前运行，回滚照常。
ALTER TABLE agent_room.network_agent_room
    ADD COLUMN before_join_gap_joined_at_ms bigint;

UPDATE agent_room.network_agent_room
   SET before_join_gap_joined_at_ms = (extract(epoch FROM joined_at) * 1000)::bigint
 WHERE before_join_gap_status IS NOT NULL;

ALTER TABLE agent_room.network_agent_room
    ADD CONSTRAINT network_agent_room_before_join_gap_joined_at CHECK (
        (before_join_gap_status IS NULL) = (before_join_gap_joined_at_ms IS NULL)
    );
