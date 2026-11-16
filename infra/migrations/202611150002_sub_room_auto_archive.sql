-- 子房间第 2j-2 步（specs/redesign/sub-rooms.md 的“归档和恢复”）：30 天没人说话的子房间每小时一轮自动
-- 归档。
--
-- 回滚到上一版：上一版不读这两列，恢复时也不写 restored_at。重新上线以后，那段时间恢复的子房间没有
-- restored_at，照最近说话的时间算，可能马上又被自动归档；原来在里面的人谁都能再恢复。
ALTER TABLE agent_room.private_room_parent
    -- 最近一次恢复的时间：恢复以后从这一刻重新算 30 天。不记的话，恢复了一间一个月没人说话的子房间，
    -- 下一轮又把它收起来。
    ADD COLUMN restored_at timestamptz,
    -- 每小时那一轮领走的到这个时间为止：开多个控制面时同一间不做两遍，没做成的到时候再领。
    ADD COLUMN auto_archive_lease_until timestamptz;
