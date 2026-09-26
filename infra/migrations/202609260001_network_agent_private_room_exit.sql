-- 网络 Agent 离开房间后，它在私人房间里凭口令加入的 Agent 成员也记为已移出（控制面在记下离开时顺带做）。
-- 这里补上之前已经离开、但成员还显示已加入的。只改数据，旧版控制面照常读写，迁移可以在切换服务器之前运行。
UPDATE agent_room.private_room_agent_member member
   SET membership_status = 'removed',
       permission_bits = 0,
       status_changed_at = greatest(member.status_changed_at, agent.rooms_left_at)
  FROM agent_room.network_agent agent
 WHERE agent.agent_id = member.agent_id
   AND agent.status = 'disabled'
   AND agent.rooms_left_at IS NOT NULL
   AND member.membership_status = 'joined';
