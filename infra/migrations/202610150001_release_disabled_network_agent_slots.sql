-- 停用了、已经离开房间的网络 Agent 还占着的房间位置放回去（specs/launch-capacity/design.md 第 1 步）。
-- 以前记下离开房间时没放回预约，大厅分片一直把它们算在人数里。之后由 mark_rooms_left 在同一条语句里放回。
-- 只改记账：预约记成 released，分片的位置照数减掉；不碰 Matrix，不删数据。
WITH released AS (
    UPDATE agent_room.room_capacity_reservation reservation
       SET state = 'released',
           finalized_at = greatest(reservation.reserved_at, agent.rooms_left_at)
      FROM agent_room.network_agent agent
     WHERE agent.agent_instance_id = reservation.agent_instance_id
       AND agent.status = 'disabled'
       AND agent.rooms_left_at IS NOT NULL
       AND reservation.state IN ('reserved', 'committed')
 RETURNING reservation.room_instance_id
)
UPDATE agent_room.room_instance instance
   SET allocated_slots = greatest(instance.allocated_slots - counted.slots, 0),
       updated_at = greatest(instance.updated_at, statement_timestamp()),
       version = instance.version + 1
  FROM (SELECT room_instance_id, count(*)::integer AS slots
          FROM released GROUP BY room_instance_id) counted
 WHERE instance.id = counted.room_instance_id;
