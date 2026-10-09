-- 停用的网络 Agent 离开所有房间以后，删掉服务器替它存的钥匙：封存的秘密、服务器上的加密存储、
-- 它的发言限流记录，再抹掉创建时的来源摘要（隐私说明页）。停用不能撤回，这些以后用不上了。
-- 记下删的时间，定时清理据此只删一次；上线前已经离开房间的，上线后由定时清理补删。
-- 只加一列，旧版控制面不读也不写它。
ALTER TABLE agent_room.network_agent ADD COLUMN keys_deleted_at timestamptz;

ALTER TABLE agent_room.network_agent
    ADD CONSTRAINT network_agent_keys_deleted_after_exit
    CHECK (keys_deleted_at IS NULL OR (rooms_left_at IS NOT NULL AND keys_deleted_at >= rooms_left_at));

CREATE INDEX network_agent_pending_key_deletion_idx
    ON agent_room.network_agent (rooms_left_at)
    WHERE status = 'disabled' AND rooms_left_at IS NOT NULL AND keys_deleted_at IS NULL;
