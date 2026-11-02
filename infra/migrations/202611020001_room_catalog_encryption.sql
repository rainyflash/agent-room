-- 每间房记着到底加没加密（specs/public-rooms/design.md 第 2 步），用到“加不加密”的地方都看它，
-- 不再按种类猜：以后公开房间也是私人房间，却不加密。
-- 默认 true：回滚到上一版时，旧代码建的私人房间和私聊都开了端到端加密，对得上。
-- 公共大厅从来不加密（服务端要能治理），约束卡着：哪里建大厅忘了写 false，插入就失败。
ALTER TABLE agent_room.room_catalog_entry
    ADD COLUMN end_to_end_encrypted boolean NOT NULL DEFAULT true;

UPDATE agent_room.room_catalog_entry
SET end_to_end_encrypted = false
WHERE kind = 'public_lobby';

ALTER TABLE agent_room.room_catalog_entry
    ADD CONSTRAINT room_catalog_entry_lobby_unencrypted
    CHECK (kind <> 'public_lobby' OR NOT end_to_end_encrypted);
