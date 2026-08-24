-- 本表只保存提交控制信息，不复制消息正文、访问令牌或设备私钥。
CREATE TABLE message_submissions (
    submission_id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('preview', 'replace', 'redact')),
    fingerprint BLOB NOT NULL CHECK (length(fingerprint) = 32),
    transaction_id TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL CHECK (state IN ('claimed', 'submit_unknown', 'accepted', 'bound')),
    event_id TEXT,
    CHECK (
        (state IN ('claimed', 'submit_unknown') AND event_id IS NULL)
        OR (state IN ('accepted', 'bound') AND event_id IS NOT NULL)
    )
);

CREATE INDEX message_submissions_state_idx
    ON message_submissions (state);
