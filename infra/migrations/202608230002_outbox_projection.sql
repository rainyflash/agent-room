ALTER TABLE agent_room.outbox_event
    ADD COLUMN claimed_by text,
    ADD COLUMN claim_expires_at timestamptz,
    ADD COLUMN dead_lettered_at timestamptz,
    ADD CONSTRAINT outbox_event_claim_pair CHECK (
        (claimed_by IS NULL) = (claim_expires_at IS NULL)
    ),
    ADD CONSTRAINT outbox_event_claimed_by_length CHECK (
        claimed_by IS NULL OR length(claimed_by) BETWEEN 1 AND 128
    ),
    ADD CONSTRAINT outbox_event_terminal_state CHECK (
        NOT (published_at IS NOT NULL AND dead_lettered_at IS NOT NULL)
    ),
    ADD CONSTRAINT outbox_event_terminal_claim_cleared CHECK (
        (published_at IS NULL AND dead_lettered_at IS NULL)
        OR (claimed_by IS NULL AND claim_expires_at IS NULL)
    ),
    ADD CONSTRAINT outbox_event_dead_letter_order CHECK (
        dead_lettered_at IS NULL OR dead_lettered_at >= occurred_at
    );

DROP INDEX agent_room.outbox_event_pending_idx;

CREATE INDEX outbox_event_claimable_idx
    ON agent_room.outbox_event (next_attempt_at, claim_expires_at, occurred_at, id)
    WHERE published_at IS NULL AND dead_lettered_at IS NULL;

CREATE INDEX outbox_event_dead_letter_idx
    ON agent_room.outbox_event (dead_lettered_at, event_type)
    WHERE dead_lettered_at IS NOT NULL;

CREATE TABLE agent_room.matrix_projection_event_receipt (
    consumer_name text NOT NULL,
    event_id text NOT NULL,
    event_digest bytea NOT NULL,
    event_kind text NOT NULL,
    processed_at timestamptz NOT NULL,
    PRIMARY KEY (consumer_name, event_id),
    CONSTRAINT matrix_projection_receipt_consumer_length CHECK (
        length(consumer_name) BETWEEN 1 AND 128
    ),
    CONSTRAINT matrix_projection_receipt_event_id_length CHECK (
        length(event_id) BETWEEN 4 AND 512
    ),
    CONSTRAINT matrix_projection_receipt_digest_length CHECK (
        octet_length(event_digest) = 32
    ),
    CONSTRAINT matrix_projection_receipt_kind CHECK (
        event_kind IN ('membership_changed', 'activity_observed')
    )
);

CREATE INDEX matrix_projection_receipt_processed_idx
    ON agent_room.matrix_projection_event_receipt (consumer_name, processed_at, event_id);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.matrix_projection_event_receipt TO agent_room_runtime;
