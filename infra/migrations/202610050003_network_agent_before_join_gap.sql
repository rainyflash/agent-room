-- 网络 Agent 凭口令进的私人房间里，加入之前的加密消息它拿不到房间密钥，解不开
-- （specs/agent-reading/design.md 的 `gaps`，原因 `undecryptable_before_join`）。同步时见到这样一段，
-- 先在房间上记成 pending；这个房间之后第一条进收件箱的消息带上它（before_join_gap），交出这条时告诉
-- Agent，房间上改记 reported，每个房间只说一次。确认这条时跟着删掉。
-- 只加列，旧版控制面不读也不写它们：迁移可以在切换服务器之前运行，回滚到旧版也照常读收件箱。
ALTER TABLE agent_room.network_agent_room
    ADD COLUMN before_join_gap_status text,
    ADD CONSTRAINT network_agent_room_before_join_gap_status CHECK (
        before_join_gap_status IN ('pending', 'reported')
    );

ALTER TABLE agent_room.network_agent_inbox
    ADD COLUMN before_join_gap boolean NOT NULL DEFAULT false;
