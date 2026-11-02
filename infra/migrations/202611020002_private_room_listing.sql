-- 公开房间的设置（specs/public-rooms/design.md）：进不进目录、别人怎么进、新成员给什么权限、
-- 让不让没有账号的网络 Agent 进、分享码。房间第一次公开时才有这一行，之后改回私密也留着：
-- 再公开还是原来的分享码，第一次公开的时间也不变。
--
-- 不放进 room_catalog_entry：那张表很热，这几列只在房主改设置、发现页读目录时才碰。
-- 目录上的 visibility 也不改成 'public'：回滚到上一版时，旧代码读到 visibility 是 'public' 的
-- 私人房间（约束 room_catalog_entry_private_visibility 不许），这间房就整个读不出来了。
CREATE TABLE agent_room.private_room_listing (
    catalog_entry_id uuid PRIMARY KEY
        REFERENCES agent_room.room_catalog_entry(id) ON DELETE CASCADE,
    listed boolean NOT NULL,
    join_mode text NOT NULL,
    member_permissions smallint NOT NULL,
    allow_network_agents boolean NOT NULL,
    share_code text NOT NULL,
    first_listed_at timestamptz NOT NULL,
    -- 最近一次有人说话的时间，发现页排序用；由第 3 步的目录定时写，先留着空。
    last_message_at timestamptz,
    updated_by uuid NOT NULL REFERENCES agent_room.principal(id),
    updated_at timestamptz NOT NULL,
    version bigint NOT NULL,
    CONSTRAINT private_room_listing_join_mode CHECK (
        join_mode IN ('open', 'request', 'invite_only')
    ),
    -- 权限位：查看 1、发言 2、邀请 4、管理 8、自动发言 16。要有查看，不给管理，
    -- 自动发言要有发言（和 PrivateRoomPermissions 的规则一样）。
    CONSTRAINT private_room_listing_member_permissions CHECK (
        member_permissions BETWEEN 1 AND 31
        AND member_permissions & 1 = 1
        AND member_permissions & 8 = 0
        AND (member_permissions & 16 = 0 OR member_permissions & 2 = 2)
    ),
    CONSTRAINT private_room_listing_share_code_format CHECK (
        share_code ~ '^[A-Za-z0-9]{6}$'
    ),
    CONSTRAINT private_room_listing_version_positive CHECK (version > 0),
    CONSTRAINT private_room_listing_timestamp_order CHECK (updated_at >= first_listed_at)
);

CREATE UNIQUE INDEX private_room_listing_share_code_unique
    ON agent_room.private_room_listing (share_code);

CREATE INDEX private_room_listing_listed_idx
    ON agent_room.private_room_listing (last_message_at DESC NULLS LAST)
    WHERE listed;
