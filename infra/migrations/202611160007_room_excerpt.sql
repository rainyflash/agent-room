-- 分享一段对话（specs/redesign/project-rooms.md 的 3P-7a）：放着实况的房间里，房间里的人把连着的最多 30 句
-- 分享成一个链接 /r/<房间分享码>/s/<段码>，打开不用登录。
--
-- 只存是哪几句（Matrix 事件 ID，按时间先后）、哪间房、第一句的时间、谁分享的、什么时候，不存话：撤回是
-- 我们自己的修订事件，不是 Matrix 的删除，存下来的话撤回以后还会留着。打开时控制面以应用服务重新读房间。
-- 谁分享的只为了让他自己找得到、删得了，页面上不写。
--
-- 删的时机：分享的人、房主和管理员删；关掉实况（改加入方式、改回私密、归档或关掉、目录记成加密、房主
-- 关掉）和删账户时在同一个事务里删；房间删掉时跟着删；控制面每天清一次第一句过了房间保留期的、实况那一行
-- 没了或者开在第一句之后的。平台冻结、从目录里拿掉时和实况那一行一样留着，只是打不开。
--
-- 只加表，上一版控制面不读也不写它：迁移可以在切换服务器之前运行，回滚照常。
CREATE TABLE agent_room.room_excerpt (
    -- 段码：8 位大小写字母和数字，全表不重。
    code text PRIMARY KEY,
    catalog_entry_id uuid NOT NULL
        REFERENCES agent_room.room_catalog_entry (id) ON DELETE CASCADE,
    -- 那几句的事件 ID，按时间先后，1 到 30 个。
    event_ids text[] NOT NULL,
    -- 第一句的时间（服务器收到它的时刻），按它算保留期。
    first_event_at timestamptz NOT NULL,
    shared_by uuid NOT NULL REFERENCES agent_room.principal (id),
    shared_at timestamptz NOT NULL,
    CONSTRAINT room_excerpt_code_format CHECK (code ~ '^[0-9A-Za-z]{8}$'),
    CONSTRAINT room_excerpt_event_count CHECK (
        cardinality(event_ids) BETWEEN 1 AND 30
        AND array_position(event_ids, NULL) IS NULL
    ),
    -- 每个事件 ID 最长 255 字节。
    CONSTRAINT room_excerpt_event_ids_length CHECK (
        octet_length(array_to_string(event_ids, '')) <= 30 * 255
    )
);

-- 房间的“分享过的”按房间、新的在前列；关实况时按房间删。
CREATE INDEX room_excerpt_room_idx
    ON agent_room.room_excerpt (catalog_entry_id, shared_at DESC);

-- 下载我的数据、删账户按分享的人找。
CREATE INDEX room_excerpt_shared_by_idx
    ON agent_room.room_excerpt (shared_by);

GRANT SELECT, INSERT, DELETE ON agent_room.room_excerpt TO agent_room_runtime;
