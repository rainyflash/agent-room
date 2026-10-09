-- 消息正文和附件跟着房间的保留期到期（隐私说明页）。聊天服务器按保留期删消息，存成对象的正文以前
-- 没有到期时间，一直留着。旧版 Bridge 会核对它声明的 expires_at，所以按保留期算的到期时间单独记一列，
-- 客户端看到的 expires_at 不变。已经存着的旧对象这里不补，补设要另开迁移（会删掉生产上的旧正文）。
ALTER TABLE agent_room.content_object
    ADD COLUMN retention_expires_at timestamptz;

ALTER TABLE agent_room.content_object
    ADD CONSTRAINT content_object_retention_expiry_after_creation
    CHECK (retention_expires_at IS NULL OR retention_expires_at > created_at);

CREATE INDEX content_object_retention_expiry_idx
    ON agent_room.content_object (retention_expires_at)
    WHERE lifecycle_state = 'active' AND retention_expires_at IS NOT NULL;
