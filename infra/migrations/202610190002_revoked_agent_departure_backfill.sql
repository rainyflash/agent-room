-- 撤销电脑时它的 Agent 离开房间第 2b 步（specs/computer-revocation/design.md）：上线以前撤销的电脑，
-- 它们的 Agent 还挂在房间里、占着位置。把这些电脑上撤销了的连接还占着已确认位置的
-- “Agent + Matrix 房间”记进离开队列，和撤销事务里记的是同一套条件；控制面的定时任务第一轮就开始做。
-- 同一个“Agent + Matrix 房间”有几台撤销了的电脑时记最后撤销的那台；已经记着的不重记。
INSERT INTO agent_room.revoked_agent_departure
       (agent_id, room_instance_id, device_id, requested_at, next_attempt_at)
SELECT DISTINCT ON (reservation.agent_id, reservation.room_instance_id)
       reservation.agent_id, reservation.room_instance_id, instance.device_id, now(), now()
  FROM agent_room.room_capacity_reservation AS reservation
  JOIN agent_room.agent_instance AS instance
    ON instance.id = reservation.agent_instance_id
  JOIN agent_room.device AS device
    ON device.id = instance.device_id
  JOIN agent_room.room_catalog_entry AS catalog
    ON catalog.id = reservation.catalog_entry_id
 WHERE device.trust_state = 'revoked'
   AND instance.revoked_at IS NOT NULL
   AND reservation.state = 'committed'
   AND catalog.kind IN ('public_lobby', 'private_room')
   AND NOT EXISTS (
       SELECT 1 FROM agent_room.network_agent AS network_agent
        WHERE network_agent.agent_instance_id = instance.id
   )
 ORDER BY reservation.agent_id, reservation.room_instance_id, device.revoked_at DESC
ON CONFLICT (agent_id, room_instance_id) DO NOTHING;
