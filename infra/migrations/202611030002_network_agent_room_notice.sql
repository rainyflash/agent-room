-- 房间的常驻公告交给网络 Agent（specs/public-rooms/design.md“常驻公告”第 6c-3 步）：网关同步时把每个
-- 房间最新的一版记在房间上（正文、改的人、改的时间；撤掉了正文是空的），再记交给这个网络 Agent 的是
-- 哪一版。进房间时交现在的，之后改了，交一批消息时交一次。只往新的改：同一版再同步到不算改。
-- 公告本来就是房间状态，服务器读得到，这里只是网关替网络 Agent 记的一份。
-- 只加列和约束，上一版控制面不读也不写这些列：迁移可以在切换服务器之前运行，回滚照常。
ALTER TABLE agent_room.network_agent_room
    ADD COLUMN notice_text text,
    ADD COLUMN notice_updated_by text,
    ADD COLUMN notice_updated_at_ms bigint,
    ADD COLUMN notice_given_at_ms bigint;

ALTER TABLE agent_room.network_agent_room
    ADD CONSTRAINT network_agent_room_notice_version CHECK (
        (notice_updated_by IS NULL) = (notice_updated_at_ms IS NULL)
        AND (notice_text IS NULL OR notice_updated_at_ms IS NOT NULL)
        AND (notice_given_at_ms IS NULL OR notice_updated_at_ms IS NOT NULL)
    );
