-- 项目房间（specs/redesign/project-rooms.md 的 3P-2a）：认领仓库、房间绑着的仓库。这一步只建表，还没有
-- 接口往里写（认领和核对是 3P-2b）。读的一边已经认它：绑着仓库的公开房间不占每个账户 1 间公开房间的
-- 名额，也不能换分享码；改回私密、归档（关掉）、删账户时在同一个事务里解绑，平台冻结、从目录里拿掉
-- 时不动。
--
-- 只加表，上一版控制面不读也不写它们：迁移可以在切换服务器之前运行，回滚照常（表里还没有行）。
--
-- 只认 github.com，host 那一列留给以后别的托管。owner 和 repo 照原样存一份给人看，再由库算一份小写的
-- 拿来比：GitHub 的名字不分大小写。名字的规矩和 /gh/ 地址一样：owner 是字母、数字、连字符，1 到 39 个，
-- 不以连字符开头；repo 是字母、数字、点、下划线、连字符，1 到 100 个，不能是 . 或 ..。
-- 不存 README、AGENTS.md 的内容。

-- 等着核对的认领。认上就删（结果记进 room_repository_binding），7 天没认上作废、删掉，所以表里的都是
-- 在等的。
CREATE TABLE agent_room.repository_claim (
    id uuid PRIMARY KEY,
    principal_id uuid NOT NULL REFERENCES agent_room.principal (id),
    host text NOT NULL,
    repository_owner text NOT NULL,
    repository_name text NOT NULL,
    repository_owner_lower text GENERATED ALWAYS AS (lower(repository_owner)) STORED,
    repository_name_lower text GENERATED ALWAYS AS (lower(repository_name)) STORED,
    -- 仓库里那一行带的码，认上以后就是这间房的分享码。
    share_code text NOT NULL,
    -- 绑一间已有的公开房间时是那间房（码就是它现在的分享码）；没有就是认上了才建房。
    target_catalog_entry_id uuid
        REFERENCES agent_room.room_catalog_entry (id) ON DELETE CASCADE,
    claimed_at timestamptz NOT NULL,
    last_checked_at timestamptz,
    -- 上次核对的结果，取哪几个值由 3P-2b 的核对定（没找到、仓库改过名、GitHub 读不了之类）。
    last_check_result text,
    check_count integer NOT NULL DEFAULT 0,
    CONSTRAINT repository_claim_host CHECK (host = 'github'),
    CONSTRAINT repository_claim_owner_format CHECK (
        repository_owner ~ '^[A-Za-z0-9][A-Za-z0-9-]{0,38}$'
    ),
    CONSTRAINT repository_claim_name_format CHECK (
        repository_name ~ '^[A-Za-z0-9._-]{1,100}$' AND repository_name NOT IN ('.', '..')
    ),
    CONSTRAINT repository_claim_share_code_format CHECK (share_code ~ '^[A-Za-z0-9]{6}$'),
    CONSTRAINT repository_claim_last_check CHECK (
        (last_checked_at IS NULL) = (last_check_result IS NULL)
    ),
    CONSTRAINT repository_claim_last_check_result_format CHECK (
        last_check_result ~ '^[a-z][a-z_]{0,63}$'
    ),
    CONSTRAINT repository_claim_check_count CHECK (check_count >= 0)
);

-- 同一个人同一个仓库只等着一个认领：再来一次回同一个。
CREATE UNIQUE INDEX repository_claim_principal_repository_unique
    ON agent_room.repository_claim (principal_id, host, repository_owner_lower, repository_name_lower);

-- 每个认领的码都不一样（绑已有房间的，同一间房同时只有一个认领在等）。和公开房间已有的码不重，
-- 由生成新码的那一步一起查。
CREATE UNIQUE INDEX repository_claim_share_code_unique
    ON agent_room.repository_claim (share_code);

CREATE INDEX repository_claim_target_idx
    ON agent_room.repository_claim (target_catalog_entry_id)
    WHERE target_catalog_entry_id IS NOT NULL;

-- 后台按认领的时间核对、7 天作废。
CREATE INDEX repository_claim_claimed_at_idx
    ON agent_room.repository_claim (claimed_at);

-- 房间绑着的仓库，一间房最多一个。房间删掉时跟着删；改回私密、归档、删账户时由控制面在同一个事务里删；
-- 平台冻结、从目录里拿掉（public_room_platform_hold）时不删，照旧占着这个仓库。
CREATE TABLE agent_room.room_repository_binding (
    catalog_entry_id uuid PRIMARY KEY
        REFERENCES agent_room.room_catalog_entry (id) ON DELETE CASCADE,
    host text NOT NULL,
    repository_owner text NOT NULL,
    repository_name text NOT NULL,
    repository_owner_lower text GENERATED ALWAYS AS (lower(repository_owner)) STORED,
    repository_name_lower text GENERATED ALWAYS AS (lower(repository_name)) STORED,
    -- 谁核对上的、什么时候、在哪个文件里找到的那一行。
    verified_by uuid NOT NULL REFERENCES agent_room.principal (id),
    verified_at timestamptz NOT NULL,
    verified_file text NOT NULL,
    CONSTRAINT room_repository_binding_host CHECK (host = 'github'),
    CONSTRAINT room_repository_binding_owner_format CHECK (
        repository_owner ~ '^[A-Za-z0-9][A-Za-z0-9-]{0,38}$'
    ),
    CONSTRAINT room_repository_binding_name_format CHECK (
        repository_name ~ '^[A-Za-z0-9._-]{1,100}$' AND repository_name NOT IN ('.', '..')
    ),
    CONSTRAINT room_repository_binding_verified_file CHECK (
        verified_file IN ('AGENTS.md', 'README.md')
    )
);

-- 一个仓库只绑一间房，大小写不论。
CREATE UNIQUE INDEX room_repository_binding_repository_unique
    ON agent_room.room_repository_binding (host, repository_owner_lower, repository_name_lower);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.repository_claim, agent_room.room_repository_binding
    TO agent_room_runtime;
