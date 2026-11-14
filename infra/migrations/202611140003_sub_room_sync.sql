-- 子房间跟着父房间变了以后，聊天服务器上还没照着做的事（specs/redesign/sub-rooms.md 的“对账”）。
--
-- 父房间一变（有人被移出、改了权限、换了房主、归档），存父房间的那个事务里就把子房间的成员记录、
-- 房主和状态一起改好；聊天服务器那一半（踢人、改发言级别、邀请新房主、锁住归档的房间）在事务外面做，
-- 这里记着哪些还没做。和改子房间在同一个事务里排进来，做成一条删一条。
--
-- 回滚到上一版：旧代码不读也不写这张表，排着的留到重新上线以后再做。
CREATE TABLE agent_room.private_room_sub_room_sync (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    catalog_entry_id uuid NOT NULL
        REFERENCES agent_room.private_room_parent (catalog_entry_id) ON DELETE CASCADE,
    -- 空：这间子房间自己的事（跟着父房间归档了）；不空：这个人在这间子房间里的成员记录变了，
    -- 照现在的记录去改聊天服务器上的他（还在的邀请、设发言级别，不在的踢出去）。
    principal_id uuid REFERENCES agent_room.principal (id),
    -- 每排一次加一：做完的那一次只删它领到的那一版，做的时候又排进来的留着。
    generation bigint NOT NULL DEFAULT 1,
    queued_at timestamptz NOT NULL,
    -- 领过几次。
    attempts integer NOT NULL DEFAULT 0,
    -- 一列管两件事：在做的时候是租约到期的时间，没做成以后是下次再试的时间；空着就是马上能领。
    next_attempt_at timestamptz,
    failure_code text,
    CONSTRAINT private_room_sub_room_sync_generation CHECK (generation > 0),
    CONSTRAINT private_room_sub_room_sync_attempts CHECK (attempts >= 0),
    CONSTRAINT private_room_sub_room_sync_failure_code_length CHECK (
        failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 128
    )
);

-- 同一间子房间里同一个人只排一条，这间房自己的事也只排一条。
CREATE UNIQUE INDEX private_room_sub_room_sync_member
    ON agent_room.private_room_sub_room_sync (catalog_entry_id, principal_id)
    WHERE principal_id IS NOT NULL;

CREATE UNIQUE INDEX private_room_sub_room_sync_room
    ON agent_room.private_room_sub_room_sync (catalog_entry_id)
    WHERE principal_id IS NULL;

-- 领的时候按能领的时间和排进来的先后找。
CREATE INDEX private_room_sub_room_sync_due
    ON agent_room.private_room_sub_room_sync (next_attempt_at NULLS FIRST, queued_at, id);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.private_room_sub_room_sync
    TO agent_room_runtime;
