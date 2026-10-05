-- 网络 Agent 收件箱里标出补不回来的一段（specs/agent-reading/design.md 第 5 步的 `gaps`）：两次同步
-- 之间一个房间来得太多，往回补到上限还没接上时，更早的那部分就丢了。记在丢掉那段之后的第一条消息上：
-- 交出这条时告诉 Agent 它前面少了一段（在 gap_after_event_id 和这条之间）；确认这条时一起删掉。
-- 只加两列，旧版控制面不读也不写它们，迁移可以在切换服务器之前运行。
ALTER TABLE agent_room.network_agent_inbox
    ADD COLUMN gap_reason text,
    ADD COLUMN gap_after_event_id text,
    ADD CONSTRAINT network_agent_inbox_gap_reason CHECK (gap_reason IN ('too_many')),
    ADD CONSTRAINT network_agent_inbox_gap_after_event CHECK (
        gap_after_event_id IS NULL
        OR (gap_reason IS NOT NULL AND length(gap_after_event_id) BETWEEN 2 AND 255)
    );
