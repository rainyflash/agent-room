-- 房主的 Agent 管房间（specs/public-rooms/design.md“房主的 Agent 管房间”）：房主和管理员给自己的一个
-- Agent 的管理授权。规则是主人写的一段话，最多 2000 字；能做的几样是 mute、kick、ban、unban 里勾上的；
-- 最长 30 天，到期就不管用了，也随时能撤。同一个房间、同一个 Agent 同一时间只有一份没撤的，新给的
-- 盖掉旧的（旧的记成被撤了）。撤了、到期了的照旧留着：之后台账里 Agent 做的事要指回按的是哪一份。
-- 给的人删账户时，他给的授权连同规则一起删掉。
CREATE TABLE agent_room.room_moderation_grant (
    id uuid PRIMARY KEY,
    catalog_entry_id uuid NOT NULL
        REFERENCES agent_room.room_catalog_entry (id) ON DELETE CASCADE,
    agent_id uuid NOT NULL REFERENCES agent_room.agent (id),
    granted_by uuid NOT NULL REFERENCES agent_room.principal (id),
    rules text NOT NULL,
    allowed_actions text[] NOT NULL,
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    revoked_by uuid REFERENCES agent_room.principal (id),
    CONSTRAINT room_moderation_grant_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT room_moderation_grant_rules_length CHECK (char_length(rules) BETWEEN 1 AND 2000),
    CONSTRAINT room_moderation_grant_actions_known CHECK (
        cardinality(allowed_actions) BETWEEN 1 AND 4
        AND allowed_actions <@ ARRAY['mute', 'kick', 'ban', 'unban']::text[]
    ),
    CONSTRAINT room_moderation_grant_lifetime CHECK (
        expires_at >= created_at + interval '1 minute'
        AND expires_at <= created_at + interval '30 days'
    ),
    CONSTRAINT room_moderation_grant_revocation CHECK (
        (revoked_at IS NULL) = (revoked_by IS NULL)
        AND (revoked_at IS NULL OR revoked_at >= created_at)
    )
);

CREATE UNIQUE INDEX room_moderation_grant_unrevoked_idx
    ON agent_room.room_moderation_grant (catalog_entry_id, agent_id)
    WHERE revoked_at IS NULL;

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.room_moderation_grant
    TO agent_room_runtime;
