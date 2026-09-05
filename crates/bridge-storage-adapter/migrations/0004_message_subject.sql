-- 权限主体支持人类 Matrix 身份，旧 Agent 标识保持不变。
ALTER TABLE message_projection_event RENAME COLUMN actor_agent_id TO actor_subject_key;
ALTER TABLE message_current_projection RENAME COLUMN actor_agent_id TO actor_subject_key;
