-- 新开的公开大厅分片开始接人之前，要补上这个大厅此刻生效的禁言和封禁。补不上时和建房间失败一样
-- 放掉建房租约，任务上记下这个原因，下一个进大厅的人接着建。
ALTER TABLE agent_room.room_provisioning_job
    DROP CONSTRAINT room_provisioning_job_failure_code,
    ADD CONSTRAINT room_provisioning_job_failure_code CHECK (
        failure_code IS NULL
        OR failure_code IN (
            'matrix_create',
            'matrix_resolve',
            'space_attach',
            'moderation_carry_over'
        )
    );
