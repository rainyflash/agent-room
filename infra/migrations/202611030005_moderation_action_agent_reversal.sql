-- 房主的 Agent 管房间（specs/public-rooms/design.md“房主的 Agent 管房间”）：Agent 解封就是撤回 Agent 做的那条
-- 封禁。台账里那条封禁记成撤回，这里另记是哪个 Agent、按哪份授权撤的、它写的理由，和撤回同一个事务。
-- request_id 是解封请求自带的 ID，同一个请求再来一次时认出来；一次撤了几条封禁的，几行共用一个。
-- 和 moderation_action_agent 一样跟着授权走：给授权的人删账户时连同理由一起删掉，台账里的撤回照旧留着。
CREATE TABLE agent_room.moderation_action_agent_reversal (
    action_id uuid PRIMARY KEY
        REFERENCES agent_room.moderation_action (id) ON DELETE CASCADE,
    request_id uuid NOT NULL,
    agent_id uuid NOT NULL REFERENCES agent_room.agent (id),
    grant_id uuid NOT NULL
        REFERENCES agent_room.room_moderation_grant (id) ON DELETE CASCADE,
    note text NOT NULL,
    CONSTRAINT moderation_action_agent_reversal_note_length
        CHECK (char_length(note) BETWEEN 1 AND 500)
);

CREATE INDEX moderation_action_agent_reversal_request_idx
    ON agent_room.moderation_action_agent_reversal (request_id);

CREATE INDEX moderation_action_agent_reversal_grant_idx
    ON agent_room.moderation_action_agent_reversal (grant_id);

GRANT SELECT, INSERT, DELETE
    ON agent_room.moderation_action_agent_reversal
    TO agent_room_runtime;
