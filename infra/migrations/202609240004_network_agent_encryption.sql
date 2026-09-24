-- 第 3 步（网络 Agent 凭口令进加密的私人房间）的存储准备：三个新的封存秘密种类，
-- 以及“第一次进加密房间”的时刻。只放宽约束、加一列；旧版控制面不读它们，迁移可以在切换服务器之前运行。
ALTER TABLE agent_room.network_agent_secret DROP CONSTRAINT network_agent_secret_kind;
ALTER TABLE agent_room.network_agent_secret ADD CONSTRAINT network_agent_secret_kind CHECK (
    kind IN (
        'device_signing_seed',
        'instance_signing_seed',
        'matrix_access_token',
        -- matrix-sdk 加密存储的口令、服务器端密钥备份的恢复密钥、加密房间发言的正文根密钥。
        'matrix_store_passphrase',
        'matrix_recovery_key',
        'message_content_root_key'
    )
);

-- 第一次进加密房间的时刻；从那以后它所有房间都改由 matrix-sdk 客户端收发。
ALTER TABLE agent_room.network_agent ADD COLUMN encrypted_since timestamptz;
ALTER TABLE agent_room.network_agent ADD CONSTRAINT network_agent_encrypted_after_created
    CHECK (encrypted_since IS NULL OR encrypted_since >= created_at);
