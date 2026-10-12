-- 私人房间是哪个 Agent 建的（specs/agent-rooms/design.md）。人建的为空；Agent 只管这种房间。
-- Agent 不会被物理删除（只改 lifecycle_state），外键不用级联。
ALTER TABLE agent_room.private_room_state
    ADD COLUMN created_by_agent_id uuid REFERENCES agent_room.agent(id);
