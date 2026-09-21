-- 带尝试号的设备刷新轮换。客户端在刷新结果未知时用同一尝试号重试，
-- 服务端据此重放同一对新令牌，而不是把这次重试判成刷新令牌重用。
-- replay_salt 只有和请求里的旧刷新令牌明文一起才能重新派生新令牌；
-- 新令牌再次轮换后重放不再可能，盐随即清空。
CREATE TABLE agent_room.device_refresh_attempt (
    refresh_token_id uuid PRIMARY KEY
        REFERENCES agent_room.device_refresh_token(id) ON DELETE CASCADE,
    attempt_id uuid NOT NULL,
    successor_refresh_token_id uuid NOT NULL UNIQUE
        REFERENCES agent_room.device_refresh_token(id) ON DELETE CASCADE,
    access_token_id uuid NOT NULL
        REFERENCES agent_room.device_access_token(id) ON DELETE CASCADE,
    replay_salt bytea,
    created_at timestamptz NOT NULL,
    CONSTRAINT device_refresh_attempt_id_v7 CHECK (substring(attempt_id::text, 15, 1) = '7'),
    CONSTRAINT device_refresh_attempt_not_self_successor CHECK (
        successor_refresh_token_id <> refresh_token_id
    ),
    CONSTRAINT device_refresh_attempt_salt_length CHECK (
        replay_salt IS NULL OR octet_length(replay_salt) = 32
    )
);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.device_refresh_attempt
    TO agent_room_runtime;
