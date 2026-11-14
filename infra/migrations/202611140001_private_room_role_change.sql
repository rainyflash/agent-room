-- 身份变动记录（specs/redesign/room-roles.md 的“记录”）：谁、什么时候、把谁、从什么身份改成了什么身份。
-- 和改成员的那次保存写在同一个事务里，只加不改。房主和管理员在房间设置里看得到。
--
-- 身份是五项权限上面的一层名字，所以两头各存成员状态、是不是房主、权限的位，和
-- private_room_membership 同一种写法。之前名单里没有这个人时，“之前”那三列都是空的。
-- 动手的人为空：不是谁动的手（房主删除账户，房间交给了管理员）。
-- 删账户时主体那一行改成“已删除的账户”，这里的记录跟着认不出是谁；房间删掉时一起删。
CREATE TABLE agent_room.private_room_role_change (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    catalog_entry_id uuid NOT NULL
        REFERENCES agent_room.private_room_state (catalog_entry_id) ON DELETE CASCADE,
    actor_principal_id uuid REFERENCES agent_room.principal (id),
    target_principal_id uuid NOT NULL REFERENCES agent_room.principal (id),
    previous_status text,
    previous_owner boolean,
    previous_permission_bits smallint,
    next_status text NOT NULL,
    next_owner boolean NOT NULL,
    next_permission_bits smallint NOT NULL,
    changed_at timestamptz NOT NULL,
    CONSTRAINT private_room_role_change_previous_whole CHECK (
        (previous_status IS NULL) = (previous_owner IS NULL)
        AND (previous_status IS NULL) = (previous_permission_bits IS NULL)
    ),
    CONSTRAINT private_room_role_change_status CHECK (
        (
            previous_status IS NULL
            OR previous_status IN ('invited', 'joined', 'declined', 'removed', 'banned')
        )
        AND next_status IN ('invited', 'joined', 'declined', 'removed', 'banned')
    ),
    CONSTRAINT private_room_role_change_permission_bits CHECK (
        (previous_permission_bits IS NULL OR previous_permission_bits BETWEEN 0 AND 31)
        AND next_permission_bits BETWEEN 0 AND 31
    )
);

CREATE INDEX private_room_role_change_by_room
    ON agent_room.private_room_role_change (catalog_entry_id, id DESC);

GRANT SELECT, INSERT
    ON agent_room.private_room_role_change
    TO agent_room_runtime;
