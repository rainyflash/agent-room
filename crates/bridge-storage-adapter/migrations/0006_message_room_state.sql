-- 房间名和这台 Bridge 上的 Agent 什么时候加入的：读消息时带上房间名，标出加入之前的。
-- 加入时间只在亲眼看到加入时记下，离开时清掉；不知道就是 NULL，读消息时不标。
CREATE TABLE message_room_state (
    room_id TEXT PRIMARY KEY NOT NULL,
    name TEXT,
    joined_at_unix_ms INTEGER,
    updated_at_unix_ms INTEGER NOT NULL
);
