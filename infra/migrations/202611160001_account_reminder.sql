-- 提醒（specs/redesign/reminders.md 的“存什么”）：每个账户的邮件提醒开关，和发到了哪儿。只新增一张表，
-- 旧控制面不受影响。没有这一行的账户照默认算：开着，没发过。
--
-- 这一版只写开关；上次发信的时间和说到哪一刻为止，等发信那一步（4R-5）再写。“是不是第一封”看
-- last_email_at 是不是空的，不另存。不存邮件内容，也不存提醒了哪几间房。删账户时跟着删。
CREATE TABLE agent_room.account_reminder (
    principal_id uuid PRIMARY KEY REFERENCES agent_room.principal(id) ON DELETE CASCADE,
    email_enabled boolean NOT NULL DEFAULT true,
    -- 本人在设置里、或者点退订改开关的时间；没改过是空的。
    email_changed_at timestamptz,
    -- 上一封提醒邮件发成的时间。
    last_email_at timestamptz,
    -- 上一封说到了哪一刻为止：下一封只说这之后来的。
    covered_until timestamptz,
    updated_at timestamptz NOT NULL
);

GRANT SELECT, INSERT, UPDATE, DELETE ON agent_room.account_reminder TO agent_room_runtime;
