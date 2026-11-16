-- 子房间里父房间须知的副本还没写的（specs/redesign/brief-and-pins.md 的“子房间里父房间的须知也算”，
-- 4N-3b）。
--
-- 进子房间的 Agent 不一定在父房间的聊天房间里，读不到父房间的须知，所以控制面以应用服务在每间子房间
-- 里写一份副本（房间状态 io.github.rainyflash.agentroom.room.parent_notice.v1）。父房间改了须知、新建
-- 了子房间时，在这里给子房间排一条（归档了的子房间也排）；做的时候照父房间此刻在聊天服务器上的须知写，
-- 写成一条删一条。这里只记哪间子房间还要写，不存须知本身。
--
-- 没加进 private_room_sub_room_sync：那张表上“一间房的事”会把整套发言级别写回去，旧代码往里排时的
-- ON CONFLICT 又认着它那几个部分唯一索引，改了它们回滚到上一版就排不进去了。
--
-- 回滚到上一版：旧代码不读也不写这张表，排着的留到重新上线以后再写。那段时间里父房间改的须知不会
-- 排进来，子房间里的副本要等父房间下一次改须知才跟上。
CREATE TABLE agent_room.private_room_parent_notice_sync (
    catalog_entry_id uuid PRIMARY KEY
        REFERENCES agent_room.private_room_parent (catalog_entry_id) ON DELETE CASCADE,
    -- 每排一次加一：做完的那一次只删它领到的那一版，做的时候又排进来的留着。
    generation bigint NOT NULL DEFAULT 1,
    queued_at timestamptz NOT NULL,
    -- 领过几次。
    attempts integer NOT NULL DEFAULT 0,
    -- 一列管两件事：在做的时候是租约到期的时间，没做成以后是下次再试的时间；空着就是马上能领。
    next_attempt_at timestamptz,
    failure_code text,
    CONSTRAINT private_room_parent_notice_sync_generation CHECK (generation > 0),
    CONSTRAINT private_room_parent_notice_sync_attempts CHECK (attempts >= 0),
    CONSTRAINT private_room_parent_notice_sync_failure_code_length CHECK (
        failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 128
    )
);

-- 领的时候按能领的时间和排进来的先后找。
CREATE INDEX private_room_parent_notice_sync_due
    ON agent_room.private_room_parent_notice_sync (next_attempt_at NULLS FIRST, queued_at);

-- 已经有的子房间都排一条：上线以后照父房间此刻的须知补上副本，父房间没有须知的看一眼就删。
INSERT INTO agent_room.private_room_parent_notice_sync (catalog_entry_id, queued_at)
SELECT catalog_entry_id, now()
  FROM agent_room.private_room_parent;

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.private_room_parent_notice_sync
    TO agent_room_runtime;
