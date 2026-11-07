-- 去掉没人用的 Matrix 投影（specs/matrix-projection/design.md 第 3 步）：
-- Alpha 89（#722）起控制面不再读写这三张表，建房也不再写 room_instance 的两列；
-- 消费者从来没有做，生产上三张表一直是空的，两列一直是 0。
-- 回滚只换镜像、不退数据库：Alpha 88 及更早的控制面建房时明写着这两列，读房间也选着它们，
-- 运行指标还读 matrix_projection_cursor，这个迁移上线以后不能再回到那些版本。

DROP TABLE agent_room.matrix_projection_event_receipt;
DROP TABLE agent_room.matrix_projection_cursor;
DROP TABLE agent_room.room_membership_projection;

-- 分配索引带着 activity_score，删列时会跟着没掉；先删掉，删完列按选分片的查法
-- （catalog_entry_id、state 过滤，按 allocated_slots、id 排）重建。
DROP INDEX agent_room.room_instance_allocation_idx;

ALTER TABLE agent_room.room_instance
    DROP CONSTRAINT room_instance_member_count,
    DROP CONSTRAINT room_instance_activity_score,
    DROP COLUMN member_count_projection,
    DROP COLUMN activity_score;

CREATE INDEX room_instance_allocation_idx
    ON agent_room.room_instance (
        catalog_entry_id,
        state,
        allocated_slots,
        id
    );
