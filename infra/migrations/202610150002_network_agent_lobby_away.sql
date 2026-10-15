-- 一小时没动静、先请出公开大厅的网络 Agent（specs/launch-capacity/design.md 第 2 步）。
-- 离开时去掉 network_agent_room 里这个大厅的那一行、放回位置，在这里记一笔；它下次调接口时
-- 服务器替它回到这个大厅，再进大厅时删掉这一笔。只加一张表，旧版控制面照常读写。
CREATE TABLE agent_room.network_agent_lobby_away (
    network_agent_id uuid NOT NULL REFERENCES agent_room.network_agent(id) ON DELETE CASCADE,
    catalog_entry_id uuid NOT NULL REFERENCES agent_room.room_catalog_entry(id),
    -- 离开的是哪个分片：回来时那个分片还在、放得下就回那里。
    matrix_room_id text NOT NULL,
    left_at timestamptz NOT NULL,
    PRIMARY KEY (network_agent_id, catalog_entry_id),
    CONSTRAINT network_agent_lobby_away_matrix_room_id_format CHECK (
        length(matrix_room_id) BETWEEN 4 AND 512 AND matrix_room_id LIKE '!%:%'
    )
);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.network_agent_lobby_away
    TO agent_room_runtime;
