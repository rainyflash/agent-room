-- 每间房记着到底加没加密（specs/public-rooms/design.md 第 2 步），用到“加不加密”的地方都看它，
-- 不再按种类猜：以后公开房间也是私人房间，却不加密。
-- 默认 true：回滚到上一版时，旧代码建的私人房间和私聊都开了端到端加密，对得上。
-- 公共大厅从来不加密（服务端要能治理），约束卡着：哪里建大厅忘了写 false，插入就失败。
-- 约束先以 NOT VALID 加上、再改已有的大厅：这张表上有延迟到提交时才查的约束触发器
-- （`room_catalog_private_integrity`），改过行以后同一个事务里再 ALTER 它，Postgres 报
-- “has pending trigger events”。NOT VALID 只是不回头查已有的行，之后的插入和修改照样查；
-- 已有的大厅紧接着就改成了 false。
ALTER TABLE agent_room.room_catalog_entry
    ADD COLUMN end_to_end_encrypted boolean NOT NULL DEFAULT true,
    ADD CONSTRAINT room_catalog_entry_lobby_unencrypted
        CHECK (kind <> 'public_lobby' OR NOT end_to_end_encrypted) NOT VALID;

UPDATE agent_room.room_catalog_entry
SET end_to_end_encrypted = false
WHERE kind = 'public_lobby';
