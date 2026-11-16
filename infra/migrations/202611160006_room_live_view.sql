-- 分享页放实况（specs/redesign/project-rooms.md 的 3P-6a）：房主或管理员打开“分享页放实况”的那一刻。
-- 有这一行、房间此刻也满足那几样（公开着、没关、没被平台拿掉、不加密、直接进、是主房间），分享页上才放
-- 这间房里开了以后说的话；早于 enabled_at 的不放。
--
-- 关了就删。改成要申请或只能邀请、改回私密（save_listing）、归档或关掉（write_room_rows）、删账户时
-- 关掉他的房间（archive_private_rooms）都在同一个事务里删，再开就是新的一刻。平台冻结、从目录里拿掉时
-- 不删，只是不放。房间删掉时跟着删。
--
-- 只加表，上一版控制面不读也不写它：迁移可以在切换服务器之前运行，回滚照常。这一版上线以前就公开着的
-- 房间没有这一行（默认关）：那时说话的人不知道会被放到网页上，房主自己开。
CREATE TABLE agent_room.room_live_view (
    catalog_entry_id uuid PRIMARY KEY
        REFERENCES agent_room.room_catalog_entry (id) ON DELETE CASCADE,
    -- 谁开的：房主或者有“管理”的成员；项目房间建好就开着，记的是认领的人。
    enabled_by uuid NOT NULL REFERENCES agent_room.principal (id),
    enabled_at timestamptz NOT NULL
);

-- 下载我的数据按开的人找。
CREATE INDEX room_live_view_enabled_by_idx
    ON agent_room.room_live_view (enabled_by);

GRANT SELECT, INSERT, UPDATE, DELETE ON agent_room.room_live_view TO agent_room_runtime;
