CREATE TABLE agent_room.content_download_window (
    principal_id uuid PRIMARY KEY,
    window_started_at timestamptz NOT NULL,
    window_ends_at timestamptz NOT NULL,
    request_count integer NOT NULL,
    byte_count bigint NOT NULL,
    last_attempt_at timestamptz NOT NULL,
    version bigint NOT NULL DEFAULT 0,
    CONSTRAINT content_download_window_principal_fk
        FOREIGN KEY (principal_id)
        REFERENCES agent_room.principal(id)
        ON DELETE CASCADE,
    CONSTRAINT content_download_window_time_order CHECK (
        window_ends_at > window_started_at
        AND last_attempt_at >= window_started_at
    ),
    CONSTRAINT content_download_window_request_count_positive CHECK (
        request_count > 0
    ),
    CONSTRAINT content_download_window_byte_count_positive CHECK (
        byte_count > 0
    ),
    CONSTRAINT content_download_window_version_nonnegative CHECK (
        version >= 0
    )
);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.content_download_window
    TO agent_room_runtime;
