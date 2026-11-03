-- 平台把公开房间从目录里拿掉（specs/public-rooms/design.md 的 3b-4b）：有一行就是拿掉了，删掉就是放回去。
-- 房主的“公开”设置不动；拿掉期间发现页、详情和分享页都不列这间房，也不能直接进、申请。
-- 原因写给房主看，平台的人写，最多 500 字。
CREATE TABLE agent_room.public_room_platform_hold (
    catalog_entry_id uuid PRIMARY KEY
        REFERENCES agent_room.room_catalog_entry (id) ON DELETE CASCADE,
    held_by uuid NOT NULL REFERENCES agent_room.principal (id),
    reason text NOT NULL DEFAULT '',
    held_at timestamptz NOT NULL,
    CONSTRAINT public_room_platform_hold_reason_length CHECK (char_length(reason) <= 500)
);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.public_room_platform_hold
    TO agent_room_runtime;
