-- 子房间第 2f-2b 步（specs/redesign/sub-rooms.md 的“对账”）：父房间里的 Agent 成员被移出时，它在子房间
-- 里的 Agent 成员记录在同一个事务里改成已移出，把它从聊天服务器上的子房间里踢出去这件事也排进这张表。
--
-- 回滚到上一版：
-- - 2c-2 到 2f-2a 那几版排“这间子房间自己的事”时写的是 `ON CONFLICT (catalog_entry_id) WHERE
--   principal_id IS NULL`，认不出改了条件的索引，会报错。那几版没有建子房间的入口，生产上没有子房间，
--   存父房间时不会往这张表里排；
-- - 那几版领到 Agent 的那一条会当成“这间子房间自己的事”，照它归没归档把聊天服务器上的房间再摆一遍，
--   做几遍都一样；
-- - Alpha 94 及更早的版本不读这张表。
ALTER TABLE agent_room.private_room_sub_room_sync
    -- 不空：这个 Agent 在这间子房间里的 Agent 成员记录变了，照现在的记录去改聊天服务器上的它（不在了的
    -- 踢出去）。和 principal_id 最多有一个。
    ADD COLUMN agent_id uuid REFERENCES agent_room.agent (id) ON DELETE CASCADE,
    ADD CONSTRAINT private_room_sub_room_sync_one_subject CHECK (
        principal_id IS NULL OR agent_id IS NULL
    );

-- 这间子房间自己的事只排一条：现在得两列都空。
DROP INDEX agent_room.private_room_sub_room_sync_room;

CREATE UNIQUE INDEX private_room_sub_room_sync_room
    ON agent_room.private_room_sub_room_sync (catalog_entry_id)
    WHERE principal_id IS NULL AND agent_id IS NULL;

-- 同一间子房间里同一个 Agent 只排一条。
CREATE UNIQUE INDEX private_room_sub_room_sync_agent
    ON agent_room.private_room_sub_room_sync (catalog_entry_id, agent_id)
    WHERE agent_id IS NOT NULL;
