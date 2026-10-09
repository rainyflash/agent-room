-- 限时治理动作到期解除的领取和重试（specs/moderation/timed-mute-expiry.md）。
-- expiry_next_attempt_at 一列管两件事：在做的时候是租约到期的时间，撤不成以后是下次重试的时间；
-- 空着就是到期马上能领。expiry_attempts 是领过几次，排下次时拿它认是不是自己领的那次，退避也按它算。
-- 只加列和索引，旧版控制面不读也不写这几列，迁移可以在切换服务器之前运行。
ALTER TABLE agent_room.moderation_action
    ADD COLUMN expiry_attempts integer NOT NULL DEFAULT 0,
    ADD COLUMN expiry_next_attempt_at timestamptz,
    ADD COLUMN expiry_failure_code text,
    ADD CONSTRAINT moderation_action_expiry_attempts CHECK (expiry_attempts >= 0),
    ADD CONSTRAINT moderation_action_expiry_failure_code_length CHECK (
        expiry_failure_code IS NULL OR length(expiry_failure_code) BETWEEN 1 AND 128
    );

CREATE INDEX moderation_action_expiry_due_idx
    ON agent_room.moderation_action (expires_at)
    WHERE status = 'applied' AND expires_at IS NOT NULL;
