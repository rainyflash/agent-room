-- 房主的 Agent 管房间（specs/public-rooms/design.md“房主的 Agent 管房间”）：Agent 按管理授权动手时，台账里
-- 照旧记一条治理动作（执行人是给授权的人），这里另记是他的哪个 Agent、按哪份授权、它写的理由。单独一张表，
-- 不往 moderation_action 加列：台账那张表读写的地方多，旧版控制面回滚时也不用管新列。
-- 理由跟着授权走：给授权的人删账户时，他给的授权连同这里的理由一起删掉，台账里那条动作照旧留着。
CREATE TABLE agent_room.moderation_action_agent (
    action_id uuid PRIMARY KEY
        REFERENCES agent_room.moderation_action (id) ON DELETE CASCADE,
    agent_id uuid NOT NULL REFERENCES agent_room.agent (id),
    grant_id uuid NOT NULL
        REFERENCES agent_room.room_moderation_grant (id) ON DELETE CASCADE,
    note text NOT NULL,
    CONSTRAINT moderation_action_agent_note_length CHECK (char_length(note) BETWEEN 1 AND 500)
);

CREATE INDEX moderation_action_agent_grant_idx
    ON agent_room.moderation_action_agent (grant_id);

GRANT SELECT, INSERT, DELETE
    ON agent_room.moderation_action_agent
    TO agent_room_runtime;
