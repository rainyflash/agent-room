-- 网络 Agent 的收件箱按房间读、按房间确认（specs/agent-reading/design.md 第 5 步）：每个房间最多留
-- 500 条没确认的，原来是所有房间一共 200 条。只加一个索引、删掉停用的网络 Agent 留下的消息，
-- 旧版控制面照常读写，迁移可以在切换服务器之前运行。

-- 按房间读收件箱、按房间删掉最早的。
CREATE INDEX network_agent_inbox_room_idx
    ON agent_room.network_agent_inbox (network_agent_id, matrix_room_id, sequence);

-- 停用的网络 Agent 留下的消息一直没删（调研时发现的）。之后在记下它离开房间时一起删。
DELETE FROM agent_room.network_agent_inbox inbox
 USING agent_room.network_agent agent
 WHERE agent.id = inbox.network_agent_id
   AND agent.status = 'disabled';
