-- 撤销电脑时它的 Agent 离开房间（specs/computer-revocation/design.md）。
-- 撤销的事务里，这台电脑上每个连接占着位置的“Agent + Matrix 房间”记一条，做完删掉。
-- next_attempt_at 一列管两件事：在做的时候是租约到期的时间，没做成以后是下次重试的时间。
-- attempts 是领过几次，排下次时拿它认是不是自己领的那次，退避也按它算。
-- 只加一张表，旧版控制面不读也不写，迁移可以在切换服务器之前运行。
CREATE TABLE agent_room.revoked_agent_departure (
    agent_id uuid NOT NULL REFERENCES agent_room.agent(id) ON DELETE CASCADE,
    room_instance_id uuid NOT NULL REFERENCES agent_room.room_instance(id) ON DELETE CASCADE,
    -- 记下它的那次撤销是哪台电脑：撤销的那次请求里只做这台电脑的。
    device_id uuid NOT NULL REFERENCES agent_room.device(id) ON DELETE CASCADE,
    requested_at timestamptz NOT NULL,
    next_attempt_at timestamptz NOT NULL,
    attempts integer NOT NULL DEFAULT 0,
    failure_code text,
    PRIMARY KEY (agent_id, room_instance_id),
    CONSTRAINT revoked_agent_departure_attempts CHECK (attempts >= 0),
    CONSTRAINT revoked_agent_departure_failure_code_length CHECK (
        failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 128
    )
);

CREATE INDEX revoked_agent_departure_due_idx
    ON agent_room.revoked_agent_departure (next_attempt_at);

CREATE INDEX revoked_agent_departure_device_idx
    ON agent_room.revoked_agent_departure (device_id, next_attempt_at);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.revoked_agent_departure
    TO agent_room_runtime;
