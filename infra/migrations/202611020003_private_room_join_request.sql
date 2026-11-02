-- 申请进公开房间（specs/public-rooms/design.md）：要申请的房间点“申请加入”，可以留一句话，房主和
-- 管理员批准或拒绝。同一个人对同一间房同时只能有一个在等的申请；没人处理的一直等着，处理完的
-- （批准、拒绝、作废、撤回）30 天后由控制面删掉，删账户时他的申请一起删。
CREATE TABLE agent_room.private_room_join_request (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    catalog_entry_id uuid NOT NULL
        REFERENCES agent_room.room_catalog_entry (id) ON DELETE CASCADE,
    requester_principal_id uuid NOT NULL
        REFERENCES agent_room.principal (id) ON DELETE CASCADE,
    message text NOT NULL DEFAULT '',
    status text NOT NULL,
    requested_at timestamptz NOT NULL,
    decided_at timestamptz,
    decided_by uuid REFERENCES agent_room.principal (id) ON DELETE SET NULL,
    CONSTRAINT private_room_join_request_status CHECK (
        status IN ('pending', 'approved', 'rejected', 'voided', 'withdrawn')
    ),
    CONSTRAINT private_room_join_request_message_length CHECK (char_length(message) <= 200),
    -- 只有在等的没有处理时间；批准和拒绝记着是谁处理的。
    CONSTRAINT private_room_join_request_decision CHECK (
        (status = 'pending') = (decided_at IS NULL)
        AND (status NOT IN ('pending', 'voided', 'withdrawn') OR decided_by IS NULL)
        AND decided_at >= requested_at
    )
);

CREATE UNIQUE INDEX private_room_join_request_one_pending
    ON agent_room.private_room_join_request (catalog_entry_id, requester_principal_id)
    WHERE status = 'pending';

CREATE INDEX private_room_join_request_pending_by_room
    ON agent_room.private_room_join_request (catalog_entry_id, requested_at)
    WHERE status = 'pending';

CREATE INDEX private_room_join_request_recent_rejection
    ON agent_room.private_room_join_request (catalog_entry_id, requester_principal_id, decided_at)
    WHERE status = 'rejected';

CREATE INDEX private_room_join_request_decided
    ON agent_room.private_room_join_request (decided_at)
    WHERE status <> 'pending';

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.private_room_join_request
    TO agent_room_runtime;
