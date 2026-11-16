-- 子房间里父房间须知的副本交给网络 Agent（specs/redesign/brief-and-pins.md“子房间里父房间的须知也算”，
-- 4N-3c）：网关同步时把子房间里的副本（room.parent_notice.v1）最新的一版记在它在的那一间子房间上（父房间
-- 的 Matrix 房间号、正文、改的人、改的时间；撤掉了正文是空的），再记交给这个网络 Agent 的是哪一版。和这一间
-- 自己的公告（202611030002）分开记、分开交。副本本来就是房间状态，服务器读得到，这里只是网关替网络 Agent
-- 记的一份，它离开这间子房间时那一行跟着删。
-- 只加列和约束，上一版控制面不读也不写这些列：迁移可以在切换服务器之前运行，回滚照常。
ALTER TABLE agent_room.network_agent_room
    ADD COLUMN parent_notice_room_id text,
    ADD COLUMN parent_notice_text text,
    ADD COLUMN parent_notice_updated_by text,
    ADD COLUMN parent_notice_updated_at_ms bigint,
    ADD COLUMN parent_notice_given_at_ms bigint;

ALTER TABLE agent_room.network_agent_room
    ADD CONSTRAINT network_agent_room_parent_notice_version CHECK (
        (parent_notice_updated_by IS NULL) = (parent_notice_updated_at_ms IS NULL)
        AND (parent_notice_room_id IS NULL) = (parent_notice_updated_at_ms IS NULL)
        AND (parent_notice_text IS NULL OR parent_notice_updated_at_ms IS NOT NULL)
        AND (parent_notice_given_at_ms IS NULL OR parent_notice_updated_at_ms IS NOT NULL)
    );
