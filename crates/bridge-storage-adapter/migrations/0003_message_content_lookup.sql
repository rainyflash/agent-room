-- 正文只能从仍有效的本地验签投影反查；独立列避免在安全边界上扫描 JSON。
ALTER TABLE message_projection_event ADD COLUMN content_id TEXT;
UPDATE message_projection_event
SET content_id = json_extract(content_json, '$.contentId')
WHERE content_json IS NOT NULL;

CREATE INDEX message_projection_event_content_idx
    ON message_projection_event (room_id, content_id, sequence);

ALTER TABLE message_current_projection ADD COLUMN content_id TEXT;
UPDATE message_current_projection
SET content_id = json_extract(content_json, '$.contentId')
WHERE content_json IS NOT NULL AND visibility = 'active';

CREATE UNIQUE INDEX message_current_room_content_idx
    ON message_current_projection (room_id, content_id)
    WHERE visibility = 'active' AND content_id IS NOT NULL;
