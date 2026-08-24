-- Task 14 之前的占位表允许空过期时间；正式缓存模型要求每份快照都可确定过期。
UPDATE agent_room.agent_card_snapshot
SET expires_at = fetched_at + INTERVAL '5 minutes'
WHERE expires_at IS NULL;

ALTER TABLE agent_room.agent_card_snapshot
    ALTER COLUMN expires_at SET NOT NULL;
