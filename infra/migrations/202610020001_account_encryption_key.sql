-- 人的设备自动签名（ADR 0011，specs/device-signing/design.md）：控制面替账户保管 Matrix 密钥存储的钥匙，
-- 任何设备登录后凭它签好自己。只新增一张表，旧控制面不受影响。

-- 每个账户一份，用部署配置里的封存密钥按用途派生的子密钥（AES-256-GCM）加密后存。
CREATE TABLE agent_room.principal_encryption_key (
    principal_id uuid PRIMARY KEY REFERENCES agent_room.principal(id) ON DELETE CASCADE,
    -- 账户数据里那把密钥存储钥匙的 ID。
    key_id text NOT NULL,
    sealed bytea NOT NULL,
    key_version smallint NOT NULL,
    updated_at timestamptz NOT NULL,
    CONSTRAINT principal_encryption_key_key_id CHECK (octet_length(key_id) BETWEEN 1 AND 255),
    -- 12 字节随机数 + 16 字节认证标签 + 32 字节钥匙。
    CONSTRAINT principal_encryption_key_sealed_length CHECK (octet_length(sealed) BETWEEN 29 AND 512),
    CONSTRAINT principal_encryption_key_key_version CHECK (key_version BETWEEN 1 AND 32767)
);

GRANT SELECT, INSERT, UPDATE, DELETE ON agent_room.principal_encryption_key TO agent_room_runtime;
