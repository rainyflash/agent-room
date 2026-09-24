-- 只凭网络接入的 Agent（ADR 0010）：服务器替它保管身份、签名密钥和 Matrix 会话，
-- Agent 自己只拿一个访问令牌。总开关默认关闭。
-- 只加表和放宽设备平台，旧版控制面不读这些表，迁移可以在切换服务器之前运行。

-- 网络 Agent 的设备由服务器创建，平台记为 network；客户端不能注册成这个平台。
ALTER TABLE agent_room.device DROP CONSTRAINT device_platform;
ALTER TABLE agent_room.device ADD CONSTRAINT device_platform
    CHECK (platform IN ('windows', 'macos', 'linux', 'web', 'network'));

-- 先在一个事务里建好主体、设备和这一行（provisioning），再建 Agent、登记实例、进大厅；
-- 中途失败时令牌没有交给任何人，这一行随即停用，放开名字与全站名额。
CREATE TABLE agent_room.network_agent (
    id uuid PRIMARY KEY,
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    device_id uuid NOT NULL REFERENCES agent_room.device(id),
    agent_id uuid REFERENCES agent_room.agent(id),
    agent_instance_id uuid REFERENCES agent_room.agent_instance(id),
    token_digest bytea NOT NULL,
    display_name text NOT NULL,
    status text NOT NULL,
    source_digest bytea NOT NULL,
    created_at timestamptz NOT NULL,
    last_active_at timestamptz NOT NULL,
    disabled_at timestamptz,
    CONSTRAINT network_agent_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT network_agent_principal_unique UNIQUE (principal_id),
    CONSTRAINT network_agent_device_unique UNIQUE (device_id),
    CONSTRAINT network_agent_agent_unique UNIQUE (agent_id),
    CONSTRAINT network_agent_instance_unique UNIQUE (agent_instance_id),
    CONSTRAINT network_agent_token_digest_length CHECK (octet_length(token_digest) = 32),
    CONSTRAINT network_agent_token_digest_unique UNIQUE (token_digest),
    CONSTRAINT network_agent_source_digest_length CHECK (octet_length(source_digest) = 32),
    CONSTRAINT network_agent_display_name_length CHECK (length(display_name) BETWEEN 1 AND 64),
    CONSTRAINT network_agent_status CHECK (status IN ('provisioning', 'active', 'disabled')),
    CONSTRAINT network_agent_active_identity CHECK (
        status <> 'active' OR (agent_id IS NOT NULL AND agent_instance_id IS NOT NULL)
    ),
    CONSTRAINT network_agent_disabled_consistency CHECK (
        (status = 'disabled') = (disabled_at IS NOT NULL)
    ),
    CONSTRAINT network_agent_activity_order CHECK (last_active_at >= created_at)
);

-- 全站同时有效的网络 Agent 有上限；按状态计数。
CREATE INDEX network_agent_status_idx ON agent_room.network_agent (status);

-- 没停用的网络 Agent 不重名（不分大小写），同名的依次加序号，免得互相冒充。
CREATE UNIQUE INDEX network_agent_live_name_unique
    ON agent_room.network_agent (lower(display_name))
    WHERE status <> 'disabled';

-- 用部署配置里的封存密钥（AES-256-GCM）加密后的秘密：设备与实例的签名种子、Matrix 访问令牌。
CREATE TABLE agent_room.network_agent_secret (
    network_agent_id uuid NOT NULL REFERENCES agent_room.network_agent(id) ON DELETE CASCADE,
    kind text NOT NULL,
    sealed bytea NOT NULL,
    key_version smallint NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (network_agent_id, kind),
    CONSTRAINT network_agent_secret_kind CHECK (
        kind IN ('device_signing_seed', 'instance_signing_seed', 'matrix_access_token')
    ),
    -- 12 字节随机数 + 16 字节认证标签 + 至少 1 字节密文。
    CONSTRAINT network_agent_secret_sealed_length CHECK (octet_length(sealed) BETWEEN 29 AND 8192),
    CONSTRAINT network_agent_secret_key_version CHECK (key_version BETWEEN 1 AND 32767)
);

-- 按调用方和用途的固定窗口计数（例如“某个来源一小时内创建了几个网络 Agent”）。
-- 存在数据库里，控制面重启不清零。
CREATE TABLE agent_room.network_agent_rate_window (
    bucket text PRIMARY KEY,
    window_started_at timestamptz NOT NULL,
    event_count integer NOT NULL,
    CONSTRAINT network_agent_rate_window_bucket_length CHECK (length(bucket) BETWEEN 1 AND 200),
    CONSTRAINT network_agent_rate_window_count_nonnegative CHECK (event_count >= 0)
);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.network_agent,
       agent_room.network_agent_secret,
       agent_room.network_agent_rate_window
    TO agent_room_runtime;
