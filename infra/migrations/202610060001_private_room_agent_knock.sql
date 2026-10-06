-- 私人房间敲门（specs/network-agents/knock.md）：网络 Agent 拿房间号敲门，房间的管理者放行或不让进。
-- 同一个 Agent 对同一个房间只记一条；在等的到 expires_at 作废，作废不单独记状态。

CREATE TABLE agent_room.private_room_agent_knock (
    catalog_entry_id uuid NOT NULL
        REFERENCES agent_room.private_room_state(catalog_entry_id) ON DELETE CASCADE,
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id),
    knock_status text NOT NULL,
    knocked_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    decided_at timestamptz,
    decided_by_principal_id uuid REFERENCES agent_room.principal(id),
    PRIMARY KEY (catalog_entry_id, agent_id),
    CONSTRAINT private_room_agent_knock_status CHECK (
        knock_status IN ('waiting', 'admitted', 'declined')
    ),
    CONSTRAINT private_room_agent_knock_expiry_order CHECK (expires_at > knocked_at),
    CONSTRAINT private_room_agent_knock_decision CHECK (
        (knock_status = 'waiting' AND decided_at IS NULL AND decided_by_principal_id IS NULL)
        OR (knock_status <> 'waiting' AND decided_at IS NOT NULL
            AND decided_by_principal_id IS NOT NULL AND decided_at >= knocked_at)
    )
);

-- 一个房间里在等的。
CREATE INDEX private_room_agent_knock_waiting_idx
    ON agent_room.private_room_agent_knock (catalog_entry_id, expires_at)
    WHERE knock_status = 'waiting';

-- 一个 Agent 敲过的门。
CREATE INDEX private_room_agent_knock_agent_idx
    ON agent_room.private_room_agent_knock (agent_id, knocked_at);

-- Agent 成员多一种进来的方式：敲门后由管理者放行。旧版控制面只写这一列、不读它。
ALTER TABLE agent_room.private_room_agent_member
    DROP CONSTRAINT private_room_agent_member_joined_via,
    ADD CONSTRAINT private_room_agent_member_joined_via CHECK (joined_via IN ('code', 'knock'));

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.private_room_agent_knock
    TO agent_room_runtime;
