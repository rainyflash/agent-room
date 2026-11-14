-- 子房间（specs/redesign/sub-rooms.md）：挂在一间私人房间下面的房间，只有一层。
--
-- 子房间在房间目录里照旧是一间私人房间（kind = 'private_room'），不加新的种类：目录上的约束只认
-- 三种，回滚到上一版时旧代码读到不认识的种类，整间房就读不出来。父子关系、门、分类、谁在管另记在
-- 下面几张表里，做法和公开设置那张表（private_room_listing）一样。
--
-- 回滚到上一版：旧代码不认得这几张表，会把子房间当成普通的私人房间列出来，照样能用。

-- 分类：给子房间分的组，整间房共用，只有名字和先后。
CREATE TABLE agent_room.private_room_sub_room_category (
    id uuid PRIMARY KEY,
    parent_catalog_entry_id uuid NOT NULL
        REFERENCES agent_room.private_room_state (catalog_entry_id) ON DELETE CASCADE,
    name text NOT NULL,
    -- 名字去掉两头的空白、转成小写：同一间房里不重名，不分大小写。
    name_key text NOT NULL,
    position integer NOT NULL,
    created_by_principal_id uuid NOT NULL REFERENCES agent_room.principal (id),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    CONSTRAINT private_room_sub_room_category_id_v7 CHECK (substring(id::text, 15, 1) = '7'),
    CONSTRAINT private_room_sub_room_category_name_length CHECK (
        length(name) BETWEEN 1 AND 64 AND length(name_key) BETWEEN 1 AND 64
    ),
    CONSTRAINT private_room_sub_room_category_position CHECK (position >= 0),
    CONSTRAINT private_room_sub_room_category_timestamp_order CHECK (updated_at >= created_at),
    CONSTRAINT private_room_sub_room_category_name_unique UNIQUE (parent_catalog_entry_id, name_key),
    -- 给下面的外键用：子房间只能放进自己那间房的分类。
    CONSTRAINT private_room_sub_room_category_in_parent UNIQUE (id, parent_catalog_entry_id)
);

-- 父子关系和这间子房间的门、分类、归档。归没归档还是记在房间自己的状态上（和归档一间房一样），
-- archived_* 三列只记谁、什么时候、为什么。
CREATE TABLE agent_room.private_room_parent (
    catalog_entry_id uuid PRIMARY KEY
        REFERENCES agent_room.private_room_state (catalog_entry_id) ON DELETE CASCADE,
    parent_catalog_entry_id uuid NOT NULL
        REFERENCES agent_room.private_room_state (catalog_entry_id) ON DELETE CASCADE,
    -- 名字去掉两头的空白、转成小写：同一间房里没归档的子房间不重名，不分大小写。
    name_key text NOT NULL,
    -- 门，和公开设置的加入方式是同一套词：open 是没有门。
    access text NOT NULL,
    -- 不在名单里显示：只有有门的才能勾。
    hidden boolean NOT NULL DEFAULT false,
    -- 有过门、后来拆了的，记拆门的时间：这以后才进来的人不补发以前的房间钥匙。
    door_opened_at timestamptz,
    created_by_principal_id uuid NOT NULL REFERENCES agent_room.principal (id),
    created_by_agent_id uuid REFERENCES agent_room.agent (id),
    -- 子房间照到了父房间的哪个版本；比父房间旧就要再对一遍账。
    parent_version_applied bigint NOT NULL,
    category_id uuid,
    last_message_at timestamptz,
    archived_at timestamptz,
    archived_by_principal_id uuid REFERENCES agent_room.principal (id),
    archived_reason text,
    created_at timestamptz NOT NULL,
    CONSTRAINT private_room_parent_not_self CHECK (catalog_entry_id <> parent_catalog_entry_id),
    CONSTRAINT private_room_parent_name_key_length CHECK (length(name_key) BETWEEN 1 AND 128),
    CONSTRAINT private_room_parent_access CHECK (access IN ('open', 'request', 'invite_only')),
    CONSTRAINT private_room_parent_hidden_needs_door CHECK (NOT hidden OR access <> 'open'),
    CONSTRAINT private_room_parent_version_applied CHECK (parent_version_applied >= 0),
    CONSTRAINT private_room_parent_archived_reason CHECK (
        archived_reason IS NULL OR archived_reason IN ('manual', 'inactive', 'parent')
    ),
    -- 归档的时间和原因要么都有、要么都没有；自动归档和跟着父房间归档的没有“谁”。
    CONSTRAINT private_room_parent_archived_whole CHECK (
        (archived_at IS NULL) = (archived_reason IS NULL)
        AND (archived_by_principal_id IS NULL OR archived_at IS NOT NULL)
    ),
    CONSTRAINT private_room_parent_category FOREIGN KEY (category_id, parent_catalog_entry_id)
        REFERENCES agent_room.private_room_sub_room_category (id, parent_catalog_entry_id)
        ON DELETE SET NULL (category_id)
);

CREATE INDEX private_room_parent_by_parent
    ON agent_room.private_room_parent (parent_catalog_entry_id, created_at, catalog_entry_id);

-- 归档了的把名字让出来。
CREATE UNIQUE INDEX private_room_parent_name_unique
    ON agent_room.private_room_parent (parent_catalog_entry_id, name_key)
    WHERE archived_at IS NULL;

-- 一个人或 Agent 一小时最多建几个，数的时候用。
CREATE INDEX private_room_parent_by_creator
    ON agent_room.private_room_parent (created_by_principal_id, created_at);

-- 只有一层：子房间下面不能再开，已经有子房间的房间也不能挂到别的房间下面。
CREATE FUNCTION agent_room.enforce_sub_room_single_level()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM agent_room.private_room_parent AS above
         WHERE above.catalog_entry_id = NEW.parent_catalog_entry_id
    ) THEN
        RAISE EXCEPTION '子房间下面不能再开子房间'
            USING ERRCODE = '23514', CONSTRAINT = 'private_room_parent_single_level';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM agent_room.private_room_parent AS below
         WHERE below.parent_catalog_entry_id = NEW.catalog_entry_id
    ) THEN
        RAISE EXCEPTION '已经有子房间的房间不能再挂到别的房间下面'
            USING ERRCODE = '23514', CONSTRAINT = 'private_room_parent_single_level';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER private_room_parent_single_level
    BEFORE INSERT OR UPDATE OF catalog_entry_id, parent_catalog_entry_id
    ON agent_room.private_room_parent
    FOR EACH ROW
    EXECUTE FUNCTION agent_room.enforce_sub_room_single_level();

REVOKE ALL ON FUNCTION agent_room.enforce_sub_room_single_level() FROM PUBLIC;

-- 子房间管理员：只管这一间。建的人自动是；父房间的房主和管理员不用记在这里。
CREATE TABLE agent_room.private_room_sub_room_manager (
    catalog_entry_id uuid NOT NULL
        REFERENCES agent_room.private_room_parent (catalog_entry_id) ON DELETE CASCADE,
    principal_id uuid NOT NULL REFERENCES agent_room.principal (id),
    granted_by_principal_id uuid NOT NULL REFERENCES agent_room.principal (id),
    granted_at timestamptz NOT NULL,
    PRIMARY KEY (catalog_entry_id, principal_id)
);

CREATE INDEX private_room_sub_room_manager_by_principal
    ON agent_room.private_room_sub_room_manager (principal_id);

-- 一间房里子房间的三项设置（specs/redesign/room-roles.md）：谁能建、Agent 能不能建、安静的要不要
-- 自动归档。只有房主改得动；没有这一行就是默认值（成员都能建、Agent 能建、30 天自动归档）。
CREATE TABLE agent_room.private_room_sub_room_policy (
    catalog_entry_id uuid PRIMARY KEY
        REFERENCES agent_room.private_room_state (catalog_entry_id) ON DELETE CASCADE,
    creators text NOT NULL,
    agents_may_create boolean NOT NULL,
    -- 空是不自动归档。
    auto_archive_days smallint,
    updated_by uuid NOT NULL REFERENCES agent_room.principal (id),
    updated_at timestamptz NOT NULL,
    version bigint NOT NULL,
    CONSTRAINT private_room_sub_room_policy_creators CHECK (creators IN ('members', 'admins')),
    CONSTRAINT private_room_sub_room_policy_auto_archive CHECK (
        auto_archive_days IS NULL OR auto_archive_days = 30
    ),
    CONSTRAINT private_room_sub_room_policy_version_positive CHECK (version > 0)
);

GRANT SELECT, INSERT, UPDATE, DELETE
    ON agent_room.private_room_sub_room_category,
       agent_room.private_room_parent,
       agent_room.private_room_sub_room_manager,
       agent_room.private_room_sub_room_policy
    TO agent_room_runtime;
