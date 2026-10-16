-- 这台电脑的授权跟着账户登录走（specs/desktop-sign-in/design.md）：桌面端登录着就能要一份一次性的
-- 电脑授权，交给 Bridge 注册这台电脑，不用再批准设备码。授权只存摘要，10 分钟内有效，用一次就删。
-- 记下那份桌面登录真正认证的时刻，注册出来的令牌组一年上限从它算（令牌组新加的 authenticated_at），
-- 不然拿一份快满一年的登录，就能换出一台再管一年的电脑。设备码注册的令牌组这一列空着，照旧按创建时间算。
-- 旧版控制面不读这张表、不写这一列，迁移可以在切换服务器之前运行。

CREATE TABLE agent_room.desktop_computer_grant (
    grant_digest bytea PRIMARY KEY,
    principal_id uuid NOT NULL REFERENCES agent_room.principal(id),
    authenticated_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    CONSTRAINT desktop_computer_grant_digest_length CHECK (octet_length(grant_digest) = 32),
    CONSTRAINT desktop_computer_grant_authenticated_order CHECK (authenticated_at <= created_at),
    CONSTRAINT desktop_computer_grant_expiry_order CHECK (expires_at > created_at)
);

CREATE INDEX desktop_computer_grant_expiry_idx
    ON agent_room.desktop_computer_grant (expires_at);

GRANT SELECT, INSERT, DELETE
    ON agent_room.desktop_computer_grant
    TO agent_room_runtime;

ALTER TABLE agent_room.device_token_family
    ADD COLUMN authenticated_at timestamptz,
    ADD CONSTRAINT device_token_family_authenticated_order CHECK (
        authenticated_at IS NULL OR authenticated_at <= created_at
    );
