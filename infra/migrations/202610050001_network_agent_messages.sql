-- 网络 Agent 房间里最近的消息（specs/agent-reading/design.md 第 5 步“按需查看”）：每个房间留最近
-- 500 条，收件箱确认过的也留着；按 ID 取、看前后、往前翻、补上回复的是哪条都从这里读。它自己发的
-- 也在这里（收件箱里没有）。编号和收件箱共用 network_agent.inbox_sequence，就是到达的先后。
-- 只加一张表，旧版控制面不读也不写它，迁移可以在切换服务器之前运行。
CREATE TABLE agent_room.network_agent_message (
    network_agent_id uuid NOT NULL REFERENCES agent_room.network_agent(id) ON DELETE CASCADE,
    sequence bigint NOT NULL,
    matrix_event_id text NOT NULL,
    matrix_room_id text NOT NULL,
    message_id uuid NOT NULL,
    -- 作者：Agent ID 或 human:<Matrix 用户>。编辑和撤回只认同一作者。
    actor_key text NOT NULL,
    -- 只看某个人时比对：作者的 Matrix 用户 ID，和转成小写的名字。
    actor_matrix_user_id text NOT NULL,
    actor_name_folded text NOT NULL,
    -- 提到它或回复它的；只看提到我的时用。
    mentions_me boolean NOT NULL,
    preview jsonb NOT NULL,
    received_at timestamptz NOT NULL,
    PRIMARY KEY (network_agent_id, sequence),
    CONSTRAINT network_agent_message_event_unique UNIQUE (network_agent_id, matrix_event_id),
    CONSTRAINT network_agent_message_sequence_positive CHECK (sequence > 0),
    CONSTRAINT network_agent_message_event_id_length CHECK (length(matrix_event_id) BETWEEN 2 AND 255),
    CONSTRAINT network_agent_message_actor_key_length CHECK (length(actor_key) BETWEEN 1 AND 600),
    CONSTRAINT network_agent_message_actor_length CHECK (
        length(actor_matrix_user_id) <= 512 AND length(actor_name_folded) <= 1024
    ),
    CONSTRAINT network_agent_message_matrix_room_id_format CHECK (
        length(matrix_room_id) BETWEEN 4 AND 512 AND matrix_room_id LIKE '!%:%'
    ),
    CONSTRAINT network_agent_message_preview_object CHECK (jsonb_typeof(preview) = 'object'),
    CONSTRAINT network_agent_message_preview_size CHECK (octet_length(preview::text) <= 65536)
);

-- 翻一个房间、按房间删掉最早的。
CREATE INDEX network_agent_message_room_idx
    ON agent_room.network_agent_message (network_agent_id, matrix_room_id, sequence);

-- 按消息 ID 取，修订（编辑、撤回）按消息 ID 找。
CREATE INDEX network_agent_message_message_idx
    ON agent_room.network_agent_message (network_agent_id, message_id);

-- 收件箱里还没确认的先搬过来，升级以后按 ID 也取得到。预览里作者的形状见 bridge-ipc 的
-- IpcActorSummary：Agent 的名字和 Matrix 用户 ID 在 actor.agent 下面，人的直接在 actor 下面。
INSERT INTO agent_room.network_agent_message (
    network_agent_id, sequence, matrix_event_id, matrix_room_id, message_id, actor_key,
    actor_matrix_user_id, actor_name_folded, mentions_me, preview, received_at
)
SELECT inbox.network_agent_id, inbox.sequence, inbox.matrix_event_id, inbox.matrix_room_id,
       inbox.message_id, inbox.actor_key,
       left(coalesce(inbox.preview -> 'actor' -> 'agent' ->> 'matrixUserId',
                     inbox.preview -> 'actor' ->> 'matrixUserId', ''), 512),
       left(lower(coalesce(inbox.preview -> 'actor' -> 'agent' ->> 'displayName',
                           inbox.preview -> 'actor' ->> 'displayName', '')), 1024),
       coalesce((inbox.preview ->> 'mentionsMe')::boolean, false),
       inbox.preview, inbox.received_at
  FROM agent_room.network_agent_inbox inbox
    ON CONFLICT DO NOTHING;
