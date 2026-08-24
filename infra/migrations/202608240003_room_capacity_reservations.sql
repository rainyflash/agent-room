ALTER TABLE agent_room.room_instance
    ADD COLUMN allocated_slots integer NOT NULL DEFAULT 0;

-- 迁移前已经投影到 Matrix 的成员仍然占用容量，避免升级瞬间超卖。
UPDATE agent_room.room_instance
SET allocated_slots = member_count_projection;

ALTER TABLE agent_room.room_instance
    ADD CONSTRAINT room_instance_allocated_slots CHECK (
        allocated_slots BETWEEN 0 AND hard_capacity
    ),
    ADD CONSTRAINT room_instance_catalog_pair_unique UNIQUE (id, catalog_entry_id);

ALTER TABLE agent_room.agent_instance
    ADD CONSTRAINT agent_instance_agent_pair_unique UNIQUE (id, agent_id);

DROP INDEX agent_room.room_instance_allocation_idx;

CREATE INDEX room_instance_allocation_idx
    ON agent_room.room_instance (
        catalog_entry_id,
        state,
        allocated_slots,
        activity_score DESC,
        id
    );

CREATE TABLE agent_room.room_capacity_reservation (
    id uuid PRIMARY KEY,
    catalog_entry_id uuid NOT NULL,
    room_instance_id uuid NOT NULL,
    agent_id uuid NOT NULL,
    agent_instance_id uuid NOT NULL,
    state text NOT NULL DEFAULT 'reserved',
    reserved_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    finalized_at timestamptz,
    CONSTRAINT room_capacity_reservation_id_v7 CHECK (
        substring(id::text, 15, 1) = '7'
    ),
    CONSTRAINT room_capacity_reservation_catalog_fk
        FOREIGN KEY (catalog_entry_id)
        REFERENCES agent_room.room_catalog_entry(id),
    CONSTRAINT room_capacity_reservation_room_fk
        FOREIGN KEY (room_instance_id, catalog_entry_id)
        REFERENCES agent_room.room_instance(id, catalog_entry_id),
    CONSTRAINT room_capacity_reservation_agent_fk
        FOREIGN KEY (agent_instance_id, agent_id)
        REFERENCES agent_room.agent_instance(id, agent_id),
    CONSTRAINT room_capacity_reservation_state CHECK (
        state IN ('reserved', 'committed', 'released', 'expired')
    ),
    CONSTRAINT room_capacity_reservation_time_order CHECK (
        expires_at > reserved_at
    ),
    CONSTRAINT room_capacity_reservation_finalization_consistency CHECK (
        (state = 'reserved' AND finalized_at IS NULL)
        OR (state <> 'reserved' AND finalized_at IS NOT NULL)
    ),
    CONSTRAINT room_capacity_reservation_finalization_order CHECK (
        finalized_at IS NULL OR finalized_at >= reserved_at
    )
);

-- 同一个运行实例在一个大厅中最多只能有一个待确认槽位和一个当前归属。
CREATE UNIQUE INDEX room_capacity_reservation_pending_unique
    ON agent_room.room_capacity_reservation (agent_instance_id, catalog_entry_id)
    WHERE state = 'reserved';

CREATE UNIQUE INDEX room_capacity_reservation_assignment_unique
    ON agent_room.room_capacity_reservation (agent_instance_id, catalog_entry_id)
    WHERE state = 'committed';

CREATE INDEX room_capacity_reservation_expiry_idx
    ON agent_room.room_capacity_reservation (expires_at, id)
    WHERE state = 'reserved';

CREATE INDEX room_capacity_reservation_room_idx
    ON agent_room.room_capacity_reservation (room_instance_id, state);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.room_capacity_reservation
    TO agent_room_runtime;
