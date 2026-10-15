-- 私人房间的邀请链接（specs/room-people/design.md）：拿到链接的人登录后就能进来，7 天后过期。
-- 一个房间同时只有一条，换链接就是覆盖，停用就是删掉。秘密只存摘要，另存一份封存的给有邀请权限的人
-- 再复制；没配封存密钥时两列都空着，这条链接以后就看不到了。
-- 旧版控制面不读这张表，迁移可以在切换服务器之前运行。

CREATE TABLE agent_room.private_room_invite_link (
    catalog_entry_id uuid PRIMARY KEY
        REFERENCES agent_room.private_room_state(catalog_entry_id) ON DELETE CASCADE,
    secret_digest bytea NOT NULL,
    sealed_secret bytea,
    sealed_secret_key_version smallint,
    permission_bits smallint NOT NULL,
    created_by_principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    CONSTRAINT private_room_invite_link_digest_length CHECK (octet_length(secret_digest) = 32),
    CONSTRAINT private_room_invite_link_digest_unique UNIQUE (secret_digest),
    CONSTRAINT private_room_invite_link_sealed_together CHECK (
        (sealed_secret IS NULL) = (sealed_secret_key_version IS NULL)
    ),
    CONSTRAINT private_room_invite_link_sealed_length CHECK (
        sealed_secret IS NULL OR octet_length(sealed_secret) BETWEEN 29 AND 1024
    ),
    CONSTRAINT private_room_invite_link_sealed_key_version CHECK (
        sealed_secret_key_version IS NULL OR sealed_secret_key_version >= 1
    ),
    CONSTRAINT private_room_invite_link_permission_bits CHECK (
        permission_bits BETWEEN 1 AND 31
        AND (permission_bits & 1) = 1
        AND ((permission_bits & 16) = 0 OR (permission_bits & 2) = 2)
    ),
    CONSTRAINT private_room_invite_link_expiry_order CHECK (expires_at > created_at)
);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.private_room_invite_link
    TO agent_room_runtime;
